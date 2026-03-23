# instant-grep (`ig`)

Trigram-indexed regex search CLI in Rust — built for fast agent and editor-adjacent search workflows.

## Stack

Rust 1.94, edition 2024. Binary: `ig`. Installed at `~/.local/bin/ig`.

## Build & Test

```bash
cargo build --release
cargo test
cp target/release/ig ~/.local/bin/ig
```

## Architecture

Sparse n-grams (port of GitHub Blackbird / danlark1/sparse_ngrams) with covering algorithm. Index stored in `.ig/` at project root (lexicon.bin + postings.bin + metadata.bin, all mmap'd).

Pipeline: `regex → regex-syntax Extractor → covering sparse n-grams → hash table lookup → posting list intersection → parallel regex verification`

## Key files

- `src/index/ngram.rs` — core algorithm (hash_bigram, build_all_ngrams, build_covering_ngrams)
- `src/index/writer.rs` — index build pipeline
- `src/index/reader.rs` — index query (mmap + hash table)
- `src/query/extract.rs` — regex → NgramQuery conversion
- `src/daemon.rs` — Unix socket daemon for sub-ms queries

## Commands

```
ig search <pattern> [path]   # search (auto-builds index)
ig index [path]              # build/rebuild index
ig status [path]             # show stats
ig watch [path]              # auto-rebuild on file changes
ig daemon [path]             # start daemon
ig query <pattern> [path]    # query daemon
```

## Conventions

- `bun` as package manager (N/A for Rust, but keep for any JS tooling)
- Conventional Commits in English
- INDEX_VERSION must be bumped when on-disk format changes
- Tests must reproduce danlark1 test vectors for sparse n-grams
- 38 default excluded directories (node_modules, target, vendor, etc.)
