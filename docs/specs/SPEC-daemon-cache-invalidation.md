# SPEC — Invalidation du cache daemon (seal + push/pull hybride)

**Statut :** stable depuis v1.18.0.
**Versions impactées :** v1.17.2 (atomic publish initial), v1.18.0 (seal + FSEvents push).
**Code de référence :** `src/index/seal.rs`, `src/daemon.rs::TenantState::reload_if_changed`, `src/index/writer.rs`, `src/index/merge.rs`, `src/index/metadata.rs`, `src/index/overlay.rs`.

## Contexte

Le daemon multi-tenant (depuis v1.16.0) maintient un `LRU<root, Arc<TenantState>>` où chaque `TenantState` détient un `IndexReader` qui `mmap`e `lexicon.bin`, `postings.bin`, et lit `metadata.bin`. Le bug observé en v1.17.1 : le daemon servait `total_files = base_count` après un rebuild, ignorant l'overlay, jusqu'à un `ig daemon stop && start`.

Trois vecteurs concouraient au bug :

1. `OverlayReader::open(...).unwrap_or(None)` collapsait toute erreur en `Ok(None)` silencieux (overlay invisible si parse error).
2. `lexicon.bin` / `postings.bin` / `metadata.bin` étaient écrits via `File::create` (`O_TRUNC`), ce qui sous macOS laisse l'inode pré-truncate vivant pour la `mmap` du daemon — la rebuild devient invisible jusqu'à reload explicite.
3. La détection de changement était basée sur `max(metadata.bin mtime, overlay_meta.bin mtime)`. Sur APFS, deux rebuilds dans la même seconde peuvent collapser le `mtime` (granularité héritée NTP / FS coarse) — *aucun* changement détecté, *aucun* reload.

v1.17.2 a fermé les trois vecteurs. v1.18.0 a remplacé la fingerprint multi-fichiers par une primitive plus simple et plus forte : le **seal**.

---

## Le `seal` — primitive d'engagement

### Format binaire

```
.ig/seal — 16 octets little-endian
┌───────────────┬────────────────────────────┐
│ generation    │ finalized_at_nanos          │
│ u64           │ u64                         │
│ 0..8          │ 8..16                       │
└───────────────┴────────────────────────────┘
```

### API (`src/index/seal.rs`)

```rust
pub struct Seal { pub generation: u64, pub finalized_at_nanos: u64 }
pub fn read_seal(ig_dir: &Path) -> Option<Seal>;        // None si manquant ou malformé
pub fn bump_seal(ig_dir: &Path) -> Result<u64>;         // atomic: tmp + rename → new gen
pub fn current_generation(ig_dir: &Path) -> u64;        // helper, 0 si pas de seal
```

`bump_seal` :
1. Lit la génération courante (`0` si absent).
2. Construit `Seal { generation: prev + 1, finalized_at_nanos: SystemTime::now() }`.
3. Écrit 16 octets dans `seal.tmp`.
4. `fs::rename(seal.tmp, seal)` — opération atomique POSIX (un *renvoi de nom*, pas une réécriture).

---

## Contrat — invariants tenus par le writer

```
╔══════════════ INVARIANT FONDAMENTAL ══════════════╗
║                                                    ║
║  Si le daemon lit  seal.generation == N            ║
║  alors  TOUS les artefacts de la génération N      ║
║         sont déjà publiés et lisibles.             ║
║                                                    ║
╚════════════════════════════════════════════════════╝
```

Conséquence pratique : le seal est **toujours** renommé en *dernier* dans toute séquence de publish.

### Séquence de publish — `build_index` (rebuild complet)

```
src/index/writer.rs::build_index
  ├─ merge::merge_segments_streaming(postings_path)
  │    ├─ écrit postings.bin.tmp
  │    └─ fs::rename(postings.bin.tmp → postings.bin)        ┐
  ├─ merge::build_lexicon_mmap_from_file(lexicon_path)       │
  │    ├─ écrit lexicon.bin.tmp via mmap                     │  artefacts
  │    └─ fs::rename(lexicon.bin.tmp → lexicon.bin)          │
  ├─ filedata.bin / bigram_df.bin (ordres internes)          │
  ├─ metadata.write_to(ig)                                   │
  │    ├─ écrit metadata.bin.tmp                             │
  │    └─ fs::rename(metadata.bin.tmp → metadata.bin)        ┘
  └─ seal::bump_seal(ig)         ◀── DERNIER ACTE
       ├─ écrit seal.tmp (16 octets)
       └─ fs::rename(seal.tmp → seal)
```

### Séquence de publish — `incremental_overlay` (overlay)

```
src/index/writer.rs::incremental_overlay
  ├─ overlay::build_overlay
  │    ├─ écrit overlay.bin.tmp / overlay_lex.bin.tmp /
  │    │   tombstones.bin.tmp / overlay_meta.bin.tmp
  │    └─ fs::rename × 4 dans cet ordre :
  │         overlay.bin → overlay_lex.bin → tombstones.bin → overlay_meta.bin
  └─ seal::bump_seal(ig)         ◀── DERNIER ACTE
```

### Cas spécial : `Index is up to date`

`build_index` peut court-circuiter avec `eprintln!("Index is up to date")` quand aucun fichier source n'a changé. Dans ce cas, **le seal n'est PAS bumpé** — il n'y a rien à invalider.

---

## Côté daemon — pull authoritative + push best-effort

### Pull (par requête)

```rust
// src/daemon.rs::TenantState::reload_if_changed
let current = seal::read_seal(&self.ig_dir);   // 16 octets
if current != self.cached_seal {
    let new_reader = IndexReader::open(&self.ig_dir)?;
    // swap atomique sous RwLock write
    rv.reader = new_reader;
    rv.cached_seal = current;
}
```

Coût ≈ 1 µs cache-chaud (un `read(2)` de 16 octets sur APFS). Comparaison du `Seal` complet (`generation` *et* `finalized_at_nanos`) : couvre le cas pathologique où `.ig/` est wipé puis rebuild — la nouvelle génération peut redémarrer à 1 mais `finalized_at_nanos` est monotone.

### Push (FSEvents — `notify`)

```rust
// src/daemon.rs::ActiveProject::start
let mut watcher = notify::recommended_watcher(|res| {
    if event.paths.iter().any(|p| p.file_name() == Some("seal" | "seal.tmp")) {
        state.reload_tenant_if_open(root);
    }
});
watcher.watch(&ig_dir, RecursiveMode::NonRecursive);
```

Actif quand `.ig/` existe au moment de `ActiveProject::start`. Sur événement seal/seal.tmp : déclenche `reload_tenant_if_open`, qui appelle `reload_if_changed` (donc lit le seal et compare). **Push ne court-circuite pas le pull** — il l'avance dans le temps.

### Pourquoi les deux

| Couche | Latence reload | Fiabilité |
|---|---|---|
| Push (FSEvents) | ~10 ms après le rename | Bonne sur APFS local. Variable sur NFS, SMB, certaines configurations Docker bind-mount. `notify` peut coalescer ou perdre des événements sous charge. |
| Pull (16 B / query) | Au plus la prochaine query | 100 % — lecture filesystem standard, indépendante du backend. |

**Push optimise le steady-state** (rebuild externe → daemon up-to-date avant la prochaine query). **Pull ferme la boucle** quand FSEvents rate quelque chose.

---

## Diagramme de séquence — rebuild externe + query

```
  Shell A (writer)              .ig/                      Daemon                Shell B (query)
       │                          │                          │                          │
       │ ig index . ──────────────▶                          │                          │
       │                          │                          │                          │
       │   write *.tmp ──────────▶│                          │                          │
       │   rename → *.bin ───────▶│   FSEvents ─────────────▶│                          │
       │     (artefacts)          │                          │  (ignored — pas seal)    │
       │                          │                          │                          │
       │   bump_seal ─────────────▶                          │                          │
       │     write seal.tmp ─────▶│   FSEvents ─────────────▶│                          │
       │     rename → seal ──────▶│   FSEvents ─────────────▶│                          │
       │                          │                          │  ◀── seal! reload         │
       │                          │                          │     read 16 B            │
       │                          │                          │     IndexReader::open    │
       │                          │                          │     swap RwLock          │
       │                          │                          │                          │
       │                          │                          │  ◀──────────  ig query   │
       │                          │                          │  cached_seal == on-disk  │
       │                          │                          │  (pas de re-reload)      │
       │                          │                          │  process_query ─────────▶│
```

Si FSEvents rate l'événement seal (cas pathologique) :

```
  Shell A                   Daemon                    Shell B
     │                          │                          │
     │ bump_seal ────────▶      │  (FSEvents lost!)        │
     │                          │                          │
     │                          │  ◀──────────  ig query   │
     │                          │  reload_if_changed:       │
     │                          │    read seal → diff!      │
     │                          │    reload                 │
     │                          │  process_query ──────────▶│
```

Le pull récupère.

---

## Anti-patterns à éviter

1. **Ne jamais** écrire le seal **avant** la fin du publish des autres artefacts. Briser cet ordre rend l'invariant fondamental faux : un daemon pourrait lire generation N et observer un `lexicon.bin` mid-write.
2. **Ne jamais** ajouter un nouveau chemin de rebuild sans appeler `seal::bump_seal` à la fin.
3. **Ne jamais** revenir à `File::create` (truncate-in-place) pour `lexicon.bin` / `postings.bin` / `metadata.bin`. Toute modification de ces artefacts doit passer par `tmp + fs::rename`.
4. **Ne jamais** masquer une erreur d'open d'overlay en `Ok(None)` silencieux. Le précédent bug (`unwrap_or(None)` à `reader.rs:94`) a coûté des heures de debug. Logger explicitement.
5. **Ne pas** bumper le seal sur l'early-exit `Index is up to date` — rien n'a changé, un bump injustifié provoque un reload daemon inutile (cache caches vidés, pénalité légère mais évitable).

---

## Tests de régression (suite)

| Test | Fichier | Vecteur testé |
|---|---|---|
| `corrupt_overlay_meta_returns_err_not_ok_none` | `src/index/overlay.rs` | Pas de silent swallow d'erreurs overlay (anti-pattern #4). |
| `missing_overlay_meta_returns_ok_none` | `src/index/overlay.rs` | Le cas légitime "pas d'overlay" reste `Ok(None)`. |
| `missing_seal_reads_as_none` | `src/index/seal.rs` | Back-compat pre-v1.18 (pas de seal → `None`). |
| `bump_creates_and_increments` | `src/index/seal.rs` | `bump_seal` est monotone et atomique. |
| `bump_is_atomic_no_tmp_left` | `src/index/seal.rs` | Pas de `seal.tmp` orphelin après publish. |
| `corrupt_seal_reads_as_none` | `src/index/seal.rs` | Robustesse : seal corrompu → traité comme absent, le prochain bump corrige. |
| `test_seal_bumped_by_full_rebuild` | `src/daemon.rs` | `build_index` bump bien le seal. |
| `test_reload_if_changed_observes_new_generation` | `src/daemon.rs` | Le daemon reload quand le seal avance. |

---

## Évolution future

Si un nouvel artefact d'index est ajouté (par exemple `signatures.bin` pour la phase 2 de symboles) :

- L'écrire **avant** le `seal::bump_seal` final.
- Si le daemon doit le `mmap`er, l'ouvrir dans `IndexReader::open`.
- Aucune modification du contrat seal n'est nécessaire — c'est précisément l'avantage par rapport à la fingerprint v1.17.2 qui aurait demandé un nouveau champ pour chaque artefact.
