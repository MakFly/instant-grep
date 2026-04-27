/*
 * ig.c — Thin C shim that routes ig CLI calls to the long-running daemon via
 * a Unix domain socket.  For subcommands outside the hot path (index, watch,
 * etc.) or when the daemon is unreachable, it falls back to execvp("ig-rust").
 *
 * Compile: cc -O2 -Wall -Wextra -std=c11 ig.c -o ig
 * Deps:    none (POSIX + sockets only)
 */

#define _POSIX_C_SOURCE 200809L

#include "parse.h"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <sys/un.h>
#include <unistd.h>

/* ── helpers (main-only, excluded when IG_NO_MAIN is defined) ────────────── */

#ifndef IG_NO_MAIN
/* Try execv on a path if it exists and is executable. Returns only on failure. */
static void try_exec(const char *path, char **argv)
{
    if (access(path, X_OK) == 0) {
        execv(path, argv);
    }
}

static void fallback(char **argv)
{
    /* 1. $IG_BACKEND override (debug / custom installs) */
    const char *backend = getenv("IG_BACKEND");
    if (backend && *backend) {
        try_exec(backend, argv);
    }

    /* 2. User install: ~/.local/share/ig/bin/ig-rust */
    const char *home = getenv("HOME");
    if (home && *home) {
        char path[IG_MAX_PATH];
        int w = snprintf(path, sizeof(path), "%s/.local/share/ig/bin/ig-rust", home);
        if (w > 0 && (size_t)w < sizeof(path)) {
            try_exec(path, argv);
        }
    }

    /* 3. System installs */
    try_exec("/usr/local/share/ig/bin/ig-rust", argv);
    try_exec("/opt/homebrew/share/ig/bin/ig-rust", argv);

    /* 4. Legacy layout: sibling ig-rust in PATH (for upgrades-in-progress) */
    execvp("ig-rust", argv);

    /* execvp only returns on error */
    fprintf(stderr,
        "ig: cannot locate ig-rust backend.\n"
        "    Set $IG_BACKEND or reinstall: curl -fsSL https://raw.githubusercontent.com/MakFly/instant-grep/main/install.sh | bash\n");
    _exit(127);
}
#endif /* IG_NO_MAIN */

/* Escape a string for JSON: writes into dst, returns bytes written. */
static int json_escape(const char *src, char *dst, size_t dst_size)
{
    size_t i = 0;
    size_t n = 0;
    while (src[i] && n + 4 < dst_size) {
        unsigned char c = (unsigned char)src[i++];
        if (c == '"' || c == '\\') {
            dst[n++] = '\\';
            dst[n++] = (char)c;
        } else if (c == '\n') {
            dst[n++] = '\\'; dst[n++] = 'n';
        } else if (c == '\r') {
            dst[n++] = '\\'; dst[n++] = 'r';
        } else if (c == '\t') {
            dst[n++] = '\\'; dst[n++] = 't';
        } else if (c < 0x20) {
            /* \uXXXX */
            int w = snprintf(dst + n, dst_size - n, "\\u%04x", c);
            if (w < 0) break;
            n += (size_t)w;
        } else {
            dst[n++] = (char)c;
        }
    }
    dst[n] = '\0';
    return (int)n;
}

/* ── parse_argv ──────────────────────────────────────────────────────────── */

int parse_argv(int argc, char **argv, ig_args_t *out)
{
    memset(out, 0, sizeof(*out));

    /* Subcommands that stay in shim mode (hot path) */
    static const char *shim_cmds[] = { "search", "grep", "files", "count", NULL };

    int positional = 0; /* 0 = subcommand/pattern slot, 1 = path slot */
    int has_subcommand = 0;

    /* argv[0] = "ig", argv[1] might be a subcommand */
    for (int i = 1; i < argc; i++) {
        const char *a = argv[i];

        /* Flags that force passthrough */
        if (strcmp(a, "--no-daemon") == 0 ||
            strcmp(a, "--help") == 0 || strcmp(a, "-h") == 0 ||
            strcmp(a, "--version") == 0 || strcmp(a, "-V") == 0) {
            out->passthrough = 1;
            return 0;
        }

        if (strcmp(a, "-i") == 0 || strcmp(a, "--case-insensitive") == 0 ||
            strcmp(a, "--ignore-case") == 0) {
            out->case_insensitive = 1;
            continue;
        }
        if (strcmp(a, "-l") == 0 || strcmp(a, "--files") == 0 ||
            strcmp(a, "--files-with-matches") == 0) {
            out->files_only = 1;
            continue;
        }
        if (strcmp(a, "-c") == 0 || strcmp(a, "--count") == 0) {
            out->count_only = 1;
            continue;
        }
        if ((strcmp(a, "-C") == 0 || strcmp(a, "--context") == 0) && i + 1 < argc) {
            out->context_lines = atoi(argv[++i]);
            continue;
        }
        if ((strcmp(a, "-t") == 0 || strcmp(a, "--type") == 0) && i + 1 < argc) {
            out->file_type = argv[++i];
            continue;
        }

        /* Unknown flag → passthrough */
        if (a[0] == '-') {
            out->passthrough = 1;
            return 0;
        }

        /* First non-flag positional: subcommand or pattern */
        if (positional == 0) {
            /* Check if it's a known shim subcommand */
            int is_shim = 0;
            for (int j = 0; shim_cmds[j]; j++) {
                if (strcmp(a, shim_cmds[j]) == 0) {
                    is_shim = 1;
                    break;
                }
            }
            if (is_shim) {
                has_subcommand = 1;
                positional = 1; /* next positional = pattern */
                /* files subcommand sets files_only implicitly */
                if (strcmp(a, "files") == 0) out->files_only = 1;
                if (strcmp(a, "count") == 0) out->count_only = 1;
                continue;
            }
            /* Unknown subcommand that looks like a word (no dash) but isn't
             * a shim command — treat as passthrough if we haven't seen a
             * pattern yet AND the next token could be a pattern. Heuristic:
             * if i == 1 and it's not a known shim cmd, it could be an
             * ig subcommand like "index", "watch", etc. */
            if (i == 1) {
                out->passthrough = 1;
                return 0;
            }
            /* Otherwise treat as pattern */
            out->pattern = a;
            positional = 2;
        } else if (positional == 1) {
            /* After a shim subcommand: next is pattern */
            out->pattern = a;
            positional = 2;
        } else {
            /* After pattern: path */
            out->path = a;
            positional = 3;
        }
    }

    /* No argv[1] at all → passthrough (needs a pattern for shim mode, but
     * daemon also accepts no-pattern for listing. Keep it simple: require at
     * least argc > 1 for shim. */
    if (argc == 1) {
        out->passthrough = 1;
        return 0;
    }

    /* If subcommand only (e.g. "ig files") with no pattern, still valid */
    (void)has_subcommand;
    return 0;
}

/* ── resolve_root ────────────────────────────────────────────────────────── */

int resolve_root(char *buf)
{
    const char *env = getenv("IG_ROOT");
    if (env && *env) {
        strncpy(buf, env, IG_MAX_PATH - 1);
        buf[IG_MAX_PATH - 1] = '\0';
        return 0;
    }

    char cwd[IG_MAX_PATH];
    if (!getcwd(cwd, sizeof(cwd))) return -1;

    /* Walk up at most 32 levels looking for a .ig/ directory */
    char candidate[IG_MAX_PATH];
    strncpy(candidate, cwd, sizeof(candidate) - 1);
    candidate[sizeof(candidate) - 1] = '\0';

    for (int level = 0; level < 32; level++) {
        char probe[IG_MAX_PATH];
        int w = snprintf(probe, sizeof(probe), "%s/.ig", candidate);
        if (w < 0 || (size_t)w >= sizeof(probe)) break;

        if (access(probe, F_OK) == 0) {
            strncpy(buf, candidate, IG_MAX_PATH - 1);
            buf[IG_MAX_PATH - 1] = '\0';
            return 0;
        }

        /* Go up one level */
        char *slash = strrchr(candidate, '/');
        if (!slash || slash == candidate) break;
        *slash = '\0';
    }

    /* Fallback: cwd */
    strncpy(buf, cwd, IG_MAX_PATH - 1);
    buf[IG_MAX_PATH - 1] = '\0';
    return 0;
}

/* ── build_socket_path ───────────────────────────────────────────────────── */

void build_socket_path(const char *root, char *buf)
{
    /* djb2 — exact replica of src/daemon.rs:socket_path() */
    unsigned long long h = 5381ULL;
    for (const unsigned char *p = (const unsigned char *)root; *p; p++) {
        h = h * 33ULL + (unsigned long long)(*p);
    }
    snprintf(buf, IG_MAX_PATH, "/tmp/ig-%llx.sock", h);
}

/* ── build_json_request ──────────────────────────────────────────────────── */

int build_json_request(const ig_args_t *args, char *buf, size_t buf_size)
{
    char pat_esc[IG_MAX_PATH * 2];
    const char *pattern = args->pattern ? args->pattern : "";
    json_escape(pattern, pat_esc, sizeof(pat_esc));

    int n = snprintf(buf, buf_size,
        "{\"pattern\":\"%s\","
        "\"case_insensitive\":%s,"
        "\"files_only\":%s,"
        "\"count_only\":%s,"
        "\"context\":%d",
        pat_esc,
        args->case_insensitive ? "true" : "false",
        args->files_only       ? "true" : "false",
        args->count_only       ? "true" : "false",
        args->context_lines);

    if (n < 0 || (size_t)n >= buf_size) return -1;

    if (args->file_type) {
        char ft_esc[256];
        json_escape(args->file_type, ft_esc, sizeof(ft_esc));
        int m = snprintf(buf + n, buf_size - (size_t)n, ",\"type\":\"%s\"}", ft_esc);
        if (m < 0 || (size_t)m >= buf_size - (size_t)n) return -1;
        n += m;
    } else {
        if ((size_t)n + 1 >= buf_size) return -1;
        buf[n++] = '}';
        buf[n]   = '\0';
    }

    return n;
}

/* ── connect_socket + relay_response (main-only) ─────────────────────────── */

#ifndef IG_NO_MAIN
static int connect_socket(const char *sock_path)
{
    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0) return -1;

    /* 100 ms timeout on connect (via SO_SNDTIMEO / SO_RCVTIMEO) */
    struct timeval tv = { .tv_sec = 0, .tv_usec = 100000 };
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, sock_path, sizeof(addr.sun_path) - 1);

    if (connect(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(fd);
        return -1;
    }
    return fd;
}

/* ── relay_response ──────────────────────────────────────────────────────── */

/*
 * Read the daemon's JSON response line, print results to stdout, and return
 * the exit code: 0 if "error":null (or error field absent), 1 otherwise.
 *
 * The daemon sends a single JSON line followed by EOF.  We buffer it,
 * print the line, and do a minimal scan for "error":null.
 */
static int relay_response(int fd)
{
    /* Read entire response into a dynamic buffer */
    size_t cap  = 65536;
    size_t used = 0;
    char  *rbuf = malloc(cap);
    if (!rbuf) return 1;

    while (1) {
        if (used == cap) {
            cap *= 2;
            char *tmp = realloc(rbuf, cap);
            if (!tmp) { free(rbuf); return 1; }
            rbuf = tmp;
        }
        ssize_t r = read(fd, rbuf + used, cap - used);
        if (r < 0) {
            if (errno == EINTR) continue;
            free(rbuf);
            return 1;
        }
        if (r == 0) break;
        used += (size_t)r;
    }

    if (used == 0) { free(rbuf); return 1; }

    /* Write raw JSON line to stdout (daemon protocol sends one JSON line) */
    fwrite(rbuf, 1, used, stdout);
    fflush(stdout);

    /* Detect error: scan for "error":null — minimal heuristic */
    int exit_code = 1;
    /* Ensure NUL-terminated for strstr */
    if (used < cap) rbuf[used] = '\0';
    else {
        char *tmp = realloc(rbuf, cap + 1);
        if (tmp) { rbuf = tmp; rbuf[used] = '\0'; }
    }
    const char *p = strstr(rbuf, "\"error\"");
    if (p) {
        p += 7; /* skip "error" */
        while (*p == ' ' || *p == ':') p++;
        if (strncmp(p, "null", 4) == 0) exit_code = 0;
    }

    free(rbuf);
    return exit_code;
}
#endif /* IG_NO_MAIN (connect_socket + relay_response) */

/* ── main ────────────────────────────────────────────────────────────────── */

#ifndef IG_NO_MAIN
int main(int argc, char **argv)
{
    ig_args_t args;
    if (parse_argv(argc, argv, &args) != 0) {
        fallback(argv);
    }

    if (args.passthrough) {
        fallback(argv);
    }

    /* Resolve project root */
    char root[IG_MAX_PATH];
    if (resolve_root(root) != 0) {
        fallback(argv);
    }

    /* Build socket path */
    char sock_path[IG_MAX_PATH];
    build_socket_path(root, sock_path);

    /* Try to connect to daemon */
    int fd = connect_socket(sock_path);
    if (fd < 0) {
        /* Daemon not running or unreachable → fallback */
        fallback(argv);
    }

    /* Build JSON request */
    char req_buf[IG_MAX_PATH * 4];
    int req_len = build_json_request(&args, req_buf, sizeof(req_buf));
    if (req_len < 0) {
        close(fd);
        fallback(argv);
    }

    /* Append newline */
    if ((size_t)req_len + 1 < sizeof(req_buf)) {
        req_buf[req_len++] = '\n';
        req_buf[req_len]   = '\0';
    }

    /* Send request */
    ssize_t sent = 0;
    while (sent < req_len) {
        ssize_t w = write(fd, req_buf + sent, (size_t)(req_len - sent));
        if (w < 0) {
            if (errno == EINTR) continue;
            close(fd);
            fallback(argv);
        }
        sent += w;
    }

    /* Signal end of request */
    shutdown(fd, SHUT_WR);

    /* Relay response and capture exit code */
    int exit_code = relay_response(fd);
    close(fd);
    return exit_code;
}
#endif /* IG_NO_MAIN */
