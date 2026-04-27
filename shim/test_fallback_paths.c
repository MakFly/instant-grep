/*
 * test_fallback_paths.c — verify the multi-location fallback ordering in shim/ig.c.
 *
 * The shim's fallback() tries, in order:
 *   1. $IG_BACKEND
 *   2. $HOME/.local/share/ig/bin/ig-rust
 *   3. /usr/local/share/ig/bin/ig-rust   (system, not testable safely)
 *   4. /opt/homebrew/share/ig/bin/ig-rust (system, not testable safely)
 *   5. execvp("ig-rust", argv)            (PATH lookup)
 *
 * We cover:
 *   t1: $IG_BACKEND wins over share dir and PATH
 *   t2: share dir wins over PATH (when $IG_BACKEND unset)
 *   t3: $IG_BACKEND pointing to non-executable falls through to share dir
 *   t4: share dir non-executable falls through to PATH
 *   t5: empty $IG_BACKEND ignored (treated as unset)
 *
 * Each location uses a fake `ig-rust` script that exits with a unique code so
 * we can identify which path was selected.
 *
 * Compile: see Makefile (test_fallback_paths target)
 * Requires: ./ig binary present (built by `make ig`).
 */

#define _POSIX_C_SOURCE 200809L
#if defined(__APPLE__) || defined(__FreeBSD__)
#  define _DARWIN_C_SOURCE 1
#else
#  define _GNU_SOURCE
#endif

#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#define SHIM_BIN "/Users/kev/Documents/lab/sandbox/instant-grep/shim/ig"

#define EXIT_BACKEND 71  /* $IG_BACKEND script */
#define EXIT_SHARE   72  /* $HOME/.local/share/ig/bin/ig-rust script */
#define EXIT_PATH    73  /* PATH-resolved ig-rust script */

static int passed = 0;
static int total  = 0;

#define PASS(name) do { printf("PASS %s\n", name); passed++; } while (0)
#define FAIL(name, ...) do { printf("FAIL %s: ", name); printf(__VA_ARGS__); printf("\n"); } while (0)

/* ── helpers ──────────────────────────────────────────────────────────────── */

static int write_script(const char *path, int exit_code, int executable)
{
    FILE *f = fopen(path, "w");
    if (!f) return -1;
    fprintf(f, "#!/bin/sh\nexit %d\n", exit_code);
    fclose(f);
    if (executable) {
        if (chmod(path, 0755) != 0) return -1;
    } else {
        if (chmod(path, 0644) != 0) return -1;
    }
    return 0;
}

static char *mk_tmpdir(const char *prefix)
{
    char tmpl[256];
    snprintf(tmpl, sizeof(tmpl), "/tmp/%s_XXXXXX", prefix);
    char *d = mkdtemp(tmpl);
    return d ? strdup(d) : NULL;
}

static void rm_rf(const char *path)
{
    char cmd[512];
    snprintf(cmd, sizeof(cmd), "rm -rf '%s'", path);
    (void)system(cmd);
}

/* Compute /tmp/ig-<hash>.sock for IG_ROOT=/tmp and ensure it's absent. */
static void ensure_no_daemon_socket(void)
{
    unsigned long long h = 5381ULL;
    for (const unsigned char *p = (const unsigned char *)"/tmp"; *p; p++)
        h = h * 33ULL + *p;
    char sock[256];
    snprintf(sock, sizeof(sock), "/tmp/ig-%llx.sock", h);
    unlink(sock);
}

/*
 * Run shim with a controlled environment.
 * env_kv is a NULL-terminated array of "KEY=VALUE" strings.
 * Returns the child's exit code, or -1 on error.
 */
static int run_shim(const char *env_kv[])
{
    pid_t pid = fork();
    if (pid < 0) return -1;
    if (pid == 0) {
        int devnull = open("/dev/null", O_WRONLY);
        if (devnull >= 0) {
            dup2(devnull, STDOUT_FILENO);
            dup2(devnull, STDERR_FILENO);
            close(devnull);
        }
        char *argv[] = { "ig", "index", NULL };  /* non-shimmable → forces fallback */
        execve(SHIM_BIN, argv, (char *const *)env_kv);
        _exit(126);
    }
    int status;
    if (waitpid(pid, &status, 0) < 0) return -1;
    return WIFEXITED(status) ? WEXITSTATUS(status) : -1;
}

/* ── fixture builder ──────────────────────────────────────────────────────── */

typedef struct {
    char *root;          /* tmp root dir */
    char *home;          /* fake $HOME */
    char *share_dir;     /* $home/.local/share/ig/bin */
    char *share_bin;     /* $share_dir/ig-rust */
    char *backend_dir;   /* tmp dir holding the $IG_BACKEND script */
    char *backend_bin;   /* $backend_dir/backend-rust */
    char *path_dir;      /* tmp dir prepended to PATH */
    char *path_bin;      /* $path_dir/ig-rust */
} fixture_t;

static void fixture_init(fixture_t *fx)
{
    fx->root = mk_tmpdir("ig_paths");
    if (!fx->root) { fprintf(stderr, "mkdtemp failed\n"); exit(1); }

    asprintf(&fx->home, "%s/home", fx->root);
    asprintf(&fx->share_dir, "%s/.local/share/ig/bin", fx->home);
    asprintf(&fx->share_bin, "%s/ig-rust", fx->share_dir);

    asprintf(&fx->backend_dir, "%s/backend", fx->root);
    asprintf(&fx->backend_bin, "%s/backend-rust", fx->backend_dir);

    asprintf(&fx->path_dir, "%s/pathdir", fx->root);
    asprintf(&fx->path_bin, "%s/ig-rust", fx->path_dir);

    /* mkdir -p for all dirs */
    char cmd[2048];
    snprintf(cmd, sizeof(cmd),
        "mkdir -p '%s' '%s' '%s'",
        fx->share_dir, fx->backend_dir, fx->path_dir);
    (void)system(cmd);
}

static void fixture_free(fixture_t *fx)
{
    rm_rf(fx->root);
    free(fx->root);
    free(fx->home);
    free(fx->share_dir);
    free(fx->share_bin);
    free(fx->backend_dir);
    free(fx->backend_bin);
    free(fx->path_dir);
    free(fx->path_bin);
}

/* Build env array. Returns count placed in env (excluding terminating NULL). */
static int build_env(const char *env_out[], size_t cap,
                     const char *home, const char *backend,
                     const char *path_dir, int empty_backend)
{
    static char path_kv[4096];
    static char home_kv[1024];
    static char backend_kv[1024];
    static char ig_root_kv[64];

    size_t n = 0;
    if (home) {
        snprintf(home_kv, sizeof(home_kv), "HOME=%s", home);
        env_out[n++] = home_kv;
    }
    if (path_dir) {
        const char *cur = getenv("PATH");
        if (!cur) cur = "/usr/bin:/bin";
        snprintf(path_kv, sizeof(path_kv), "PATH=%s:%s", path_dir, cur);
        env_out[n++] = path_kv;
    }
    if (backend) {
        snprintf(backend_kv, sizeof(backend_kv), "IG_BACKEND=%s", backend);
        env_out[n++] = backend_kv;
    } else if (empty_backend) {
        env_out[n++] = "IG_BACKEND=";
    }
    /* IG_ROOT=/tmp keeps resolve_root predictable; daemon socket guaranteed absent */
    snprintf(ig_root_kv, sizeof(ig_root_kv), "IG_ROOT=/tmp");
    env_out[n++] = ig_root_kv;
    if (n >= cap) { fprintf(stderr, "env overflow\n"); exit(1); }
    env_out[n] = NULL;
    return (int)n;
}

/* ── tests ────────────────────────────────────────────────────────────────── */

/* t1: $IG_BACKEND points to executable → shim picks it (highest priority). */
static void t1_backend_wins(void)
{
    total++;
    const char *name = "t1_backend_env_wins";

    fixture_t fx; fixture_init(&fx);
    write_script(fx.backend_bin, EXIT_BACKEND, 1);
    write_script(fx.share_bin,   EXIT_SHARE,   1);
    write_script(fx.path_bin,    EXIT_PATH,    1);
    ensure_no_daemon_socket();

    const char *env[8];
    build_env(env, 8, fx.home, fx.backend_bin, fx.path_dir, 0);

    int code = run_shim(env);
    if (code == EXIT_BACKEND) PASS(name);
    else FAIL(name, "expected exit=%d (backend), got %d", EXIT_BACKEND, code);

    fixture_free(&fx);
}

/* t2: no $IG_BACKEND, share dir has executable → shim picks it over PATH. */
static void t2_share_wins_over_path(void)
{
    total++;
    const char *name = "t2_share_dir_wins_over_path";

    fixture_t fx; fixture_init(&fx);
    write_script(fx.share_bin, EXIT_SHARE, 1);
    write_script(fx.path_bin,  EXIT_PATH,  1);
    ensure_no_daemon_socket();

    const char *env[8];
    build_env(env, 8, fx.home, NULL, fx.path_dir, 0);

    int code = run_shim(env);
    if (code == EXIT_SHARE) PASS(name);
    else FAIL(name, "expected exit=%d (share), got %d", EXIT_SHARE, code);

    fixture_free(&fx);
}

/* t3: $IG_BACKEND is non-executable → falls through to share dir. */
static void t3_backend_nonexec_falls_through(void)
{
    total++;
    const char *name = "t3_backend_non_exec_falls_through";

    fixture_t fx; fixture_init(&fx);
    write_script(fx.backend_bin, EXIT_BACKEND, 0);  /* not executable */
    write_script(fx.share_bin,   EXIT_SHARE,   1);
    write_script(fx.path_bin,    EXIT_PATH,    1);
    ensure_no_daemon_socket();

    const char *env[8];
    build_env(env, 8, fx.home, fx.backend_bin, fx.path_dir, 0);

    int code = run_shim(env);
    if (code == EXIT_SHARE) PASS(name);
    else FAIL(name, "expected exit=%d (share, after non-exec backend), got %d", EXIT_SHARE, code);

    fixture_free(&fx);
}

/* t4: share dir binary missing/non-executable → falls through to PATH. */
static void t4_share_nonexec_falls_through_to_path(void)
{
    total++;
    const char *name = "t4_share_missing_falls_through_to_path";

    fixture_t fx; fixture_init(&fx);
    /* deliberately do NOT create share_bin */
    write_script(fx.path_bin, EXIT_PATH, 1);
    ensure_no_daemon_socket();

    const char *env[8];
    build_env(env, 8, fx.home, NULL, fx.path_dir, 0);

    int code = run_shim(env);
    if (code == EXIT_PATH) PASS(name);
    else FAIL(name, "expected exit=%d (path), got %d", EXIT_PATH, code);

    fixture_free(&fx);
}

/* t5: empty $IG_BACKEND ("") is treated as unset → continues to share dir. */
static void t5_empty_backend_treated_as_unset(void)
{
    total++;
    const char *name = "t5_empty_backend_treated_as_unset";

    fixture_t fx; fixture_init(&fx);
    write_script(fx.share_bin, EXIT_SHARE, 1);
    write_script(fx.path_bin,  EXIT_PATH,  1);
    ensure_no_daemon_socket();

    const char *env[8];
    build_env(env, 8, fx.home, NULL, fx.path_dir, 1);  /* empty IG_BACKEND */

    int code = run_shim(env);
    if (code == EXIT_SHARE) PASS(name);
    else FAIL(name, "expected exit=%d (share), got %d", EXIT_SHARE, code);

    fixture_free(&fx);
}

/* ── main ─────────────────────────────────────────────────────────────────── */

int main(void)
{
    if (access(SHIM_BIN, X_OK) != 0) {
        fprintf(stderr, "ERROR: %s not found or not executable. Run `make ig` first.\n", SHIM_BIN);
        return 1;
    }

    printf("=== test_fallback_paths ===\n");

    t1_backend_wins();
    t2_share_wins_over_path();
    t3_backend_nonexec_falls_through();
    t4_share_nonexec_falls_through_to_path();
    t5_empty_backend_treated_as_unset();

    printf("\n%d/%d tests passed\n", passed, total);
    return (passed == total) ? 0 : 1;
}
