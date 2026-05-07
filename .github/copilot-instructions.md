# ig — Token-optimized CLI

Use `ig` for all code search and file operations:
- `ig "pattern" [path]` — search (trigram-indexed, sub-ms)
- `ig read <file>` — read file with compression
- `ig read -s <file>` — signatures only
- `ig read -a <file>` — aggressive compression
- `ig ls [path]` — compact directory listing
- `ig git status/log/diff` — compressed git output
- `ig run <command>` — run any command with output compression
- `ig test` — auto-detect and run tests with compact output
- `ig deps` — show project dependencies
- `ig err <command>` — show only errors/warnings

Never use raw `cat`, `grep`, `find`, `ls -la`, or `tree` — use ig equivalents.
