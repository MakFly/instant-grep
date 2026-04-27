#ifndef IG_SHIM_PARSE_H
#define IG_SHIM_PARSE_H

#include <stddef.h>

/* Maximum path length used throughout the shim */
#define IG_MAX_PATH 4096

/* Parsed representation of the CLI arguments */
typedef struct {
    const char *pattern;       /* positional: search pattern (may be NULL for files-only) */
    const char *path;          /* positional: search root path (may be NULL → use root) */
    const char *file_type;     /* -t / --type value */
    int case_insensitive;      /* -i / --case-insensitive */
    int files_only;            /* -l / --files */
    int count_only;            /* -c / --count */
    int context_lines;         /* -C / --context N */
    int passthrough;           /* 1 → skip shim, exec ig-rust directly */
} ig_args_t;

/*
 * parse_argv — Parse argc/argv into ig_args_t.
 * Sets passthrough=1 for unknown subcommands, --no-daemon, --help, --version.
 * Returns 0 on success, -1 on usage error.
 */
int parse_argv(int argc, char **argv, ig_args_t *out);

/*
 * resolve_root — Determine the project root directory.
 * Priority: IG_ROOT env var → walk up from cwd looking for .ig/ → cwd.
 * Writes result into buf (size IG_MAX_PATH). Returns 0 on success, -1 on error.
 */
int resolve_root(char *buf);

/*
 * build_socket_path — Compute the Unix socket path for a given root.
 * Replicates the djb2 hash from src/daemon.rs:socket_path().
 * Writes "/tmp/ig-<hex>.sock" into buf (size IG_MAX_PATH).
 */
void build_socket_path(const char *root, char *buf);

/*
 * build_json_request — Serialise ig_args_t into a one-line JSON object.
 * The pattern argument overrides args->pattern (allows pre-processing).
 * Writes into buf (size buf_size). Returns number of bytes written (excl. NUL).
 */
int build_json_request(const ig_args_t *args, char *buf, size_t buf_size);

#endif /* IG_SHIM_PARSE_H */
