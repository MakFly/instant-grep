# Hermes plugin

`ig setup --agent hermes` installs `__init__.py` to
`~/.hermes/plugins/ig-rewrite/`.

The plugin rewrites `grep`/`rg`/`find`/`cat` commands to their `ig`
equivalents before Hermes executes them. Skipped silently if `~/.hermes/`
doesn't exist (pass `--auto-patch` to override).

## Disabling

```bash
ig setup --uninstall --agent hermes
```
