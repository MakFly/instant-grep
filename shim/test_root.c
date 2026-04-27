/*
 * test_root.c — Unit tests for resolve_root().
 *
 * Strategy: compile against ig_nomain.o (ig.c built with -DIG_NO_MAIN).
 * This file only includes parse.h for declarations + system headers.
 */

#define _POSIX_C_SOURCE 200809L
#define _DARWIN_C_SOURCE

#include "parse.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

/* ── helpers ──────────────────────────────────────────────────────────────── */

static int tests_run    = 0;
static int tests_passed = 0;

#define CHECK(cond) do { \
    tests_run++; \
    if (cond) { \
        tests_passed++; \
    } else { \
        fprintf(stderr, "FAIL  [%s:%d] %s\n", __FILE__, __LINE__, #cond); \
    } \
} while (0)

/* Save/restore cwd and IG_ROOT around each test. */
static char saved_cwd[IG_MAX_PATH];
static char saved_ig_root[IG_MAX_PATH];
static int  had_ig_root;

static void setup(void)
{
    if (!getcwd(saved_cwd, sizeof(saved_cwd))) {
        perror("setup: getcwd");
        exit(1);
    }
    const char *v = getenv("IG_ROOT");
    if (v) {
        had_ig_root = 1;
        strncpy(saved_ig_root, v, sizeof(saved_ig_root) - 1);
        saved_ig_root[sizeof(saved_ig_root) - 1] = '\0';
    } else {
        had_ig_root = 0;
        saved_ig_root[0] = '\0';
    }
    unsetenv("IG_ROOT");
}

static void teardown(void)
{
    if (chdir(saved_cwd) != 0) {
        perror("teardown: chdir");
    }
    if (had_ig_root) {
        setenv("IG_ROOT", saved_ig_root, 1);
    } else {
        unsetenv("IG_ROOT");
    }
}

/* Create a temp dir, optionally mkdir .ig/ inside. Returns malloced real path. */
static char *make_tmpdir(int with_ig)
{
    char tmpl[] = "/tmp/ig_root_testXXXXXX";
    char *dir = mkdtemp(tmpl);
    if (!dir) { perror("mkdtemp"); exit(1); }
    /* Resolve symlinks so path matches what getcwd() returns (macOS: /tmp → /private/tmp). */
    char resolved[IG_MAX_PATH];
    if (!realpath(dir, resolved)) { perror("realpath"); exit(1); }
    char *path = strdup(resolved);
    if (with_ig) {
        char ig_path[IG_MAX_PATH];
        snprintf(ig_path, sizeof(ig_path), "%s/.ig", path);
        if (mkdir(ig_path, 0755) != 0) { perror("mkdir .ig"); exit(1); }
    }
    return path;
}

/* Recursively remove a directory tree (max depth adequate for tests). */
static void rmdir_tree(const char *path)
{
    char cmd[IG_MAX_PATH + 16];
    snprintf(cmd, sizeof(cmd), "rm -rf %s", path);
    (void)system(cmd);
}

/* ── test cases ──────────────────────────────────────────────────────────── */

/* 1. IG_ROOT défini absolu → utilisé tel quel */
static void test_ig_root_absolute(void)
{
    setup();
    char *tmp = make_tmpdir(0);
    setenv("IG_ROOT", tmp, 1);

    char buf[IG_MAX_PATH];
    int rc = resolve_root(buf);
    CHECK(rc == 0);
    CHECK(strcmp(buf, tmp) == 0);

    rmdir_tree(tmp);
    free(tmp);
    teardown();
}

/* 2. IG_ROOT défini relatif → résolu tel quel (resolve_root copie la valeur brute) */
static void test_ig_root_relative(void)
{
    setup();
    /* resolve_root copies IG_ROOT as-is; the caller is responsible for abs. */
    setenv("IG_ROOT", "some/relative/path", 1);

    char buf[IG_MAX_PATH];
    int rc = resolve_root(buf);
    CHECK(rc == 0);
    CHECK(strcmp(buf, "some/relative/path") == 0);

    teardown();
}

/* 3. pas d'IG_ROOT, .ig/ dans cwd → cwd retourné */
static void test_no_env_ig_in_cwd(void)
{
    setup();
    char *tmp = make_tmpdir(1); /* .ig/ created */
    if (chdir(tmp) != 0) { perror("chdir"); exit(1); }

    char buf[IG_MAX_PATH];
    int rc = resolve_root(buf);
    CHECK(rc == 0);
    CHECK(strcmp(buf, tmp) == 0);

    rmdir_tree(tmp);
    free(tmp);
    teardown();
}

/* 4. pas d'IG_ROOT, .ig/ dans parent → parent retourné */
static void test_no_env_ig_in_parent(void)
{
    setup();
    char *parent = make_tmpdir(1); /* .ig/ in parent */
    char child[IG_MAX_PATH];
    snprintf(child, sizeof(child), "%s/child", parent);
    if (mkdir(child, 0755) != 0) { perror("mkdir child"); exit(1); }
    if (chdir(child) != 0) { perror("chdir child"); exit(1); }

    char buf[IG_MAX_PATH];
    int rc = resolve_root(buf);
    CHECK(rc == 0);
    CHECK(strcmp(buf, parent) == 0);

    rmdir_tree(parent);
    free(parent);
    teardown();
}

/* 5. pas d'IG_ROOT, .ig/ 5 niveaux au-dessus → trouvé */
static void test_no_env_ig_five_levels_up(void)
{
    setup();
    char *root = make_tmpdir(1); /* .ig/ here */

    /* Build 5 nested subdirectories */
    char deep[IG_MAX_PATH];
    strncpy(deep, root, sizeof(deep) - 1);
    deep[sizeof(deep) - 1] = '\0';
    for (int i = 0; i < 5; i++) {
        char next[IG_MAX_PATH];
        snprintf(next, sizeof(next), "%s/d%d", deep, i);
        if (mkdir(next, 0755) != 0) { perror("mkdir deep"); exit(1); }
        strncpy(deep, next, sizeof(deep) - 1);
    }
    if (chdir(deep) != 0) { perror("chdir deep"); exit(1); }

    char buf[IG_MAX_PATH];
    int rc = resolve_root(buf);
    CHECK(rc == 0);
    CHECK(strcmp(buf, root) == 0);

    rmdir_tree(root);
    free(root);
    teardown();
}

/* 6. cap 32 niveaux respecté — walk-up arrêté, pas de boucle infinie */
static void test_cap_32_levels(void)
{
    setup();
    /* Build a chain of 35 directories with NO .ig/ anywhere — walk-up must
     * stop at 32 and return cwd as fallback without hanging. */
    char *top = make_tmpdir(0);
    char cur[IG_MAX_PATH];
    strncpy(cur, top, sizeof(cur) - 1);
    cur[sizeof(cur) - 1] = '\0';
    for (int i = 0; i < 35; i++) {
        char next[IG_MAX_PATH];
        snprintf(next, sizeof(next), "%s/l%d", cur, i);
        if (mkdir(next, 0755) != 0) { perror("mkdir level"); exit(1); }
        strncpy(cur, next, sizeof(cur) - 1);
    }
    if (chdir(cur) != 0) { perror("chdir 35-level"); exit(1); }

    char cwd_before[IG_MAX_PATH];
    if (!getcwd(cwd_before, sizeof(cwd_before))) { perror("getcwd"); exit(1); }

    char buf[IG_MAX_PATH];
    int rc = resolve_root(buf);
    /* Must return 0 (no crash / hang) and return the cwd as fallback */
    CHECK(rc == 0);
    CHECK(strcmp(buf, cwd_before) == 0);

    rmdir_tree(top);
    free(top);
    teardown();
}

/* 7. aucun .ig/ trouvé → cwd retourné en fallback */
static void test_fallback_to_cwd(void)
{
    setup();
    char *tmp = make_tmpdir(0); /* no .ig/ */
    if (chdir(tmp) != 0) { perror("chdir"); exit(1); }

    char cwd_expect[IG_MAX_PATH];
    if (!getcwd(cwd_expect, sizeof(cwd_expect))) { perror("getcwd"); exit(1); }

    char buf[IG_MAX_PATH];
    int rc = resolve_root(buf);
    CHECK(rc == 0);
    CHECK(strcmp(buf, cwd_expect) == 0);

    rmdir_tree(tmp);
    free(tmp);
    teardown();
}

/* 8. IG_ROOT pointe vers chemin inexistant → utilisé tel quel */
static void test_ig_root_nonexistent(void)
{
    setup();
    setenv("IG_ROOT", "/tmp/ig_root_does_not_exist_xyz123", 1);

    char buf[IG_MAX_PATH];
    int rc = resolve_root(buf);
    CHECK(rc == 0);
    CHECK(strcmp(buf, "/tmp/ig_root_does_not_exist_xyz123") == 0);

    teardown();
}

/* 9. IG_ROOT="" (vide) → walk-up puis fallback cwd */
static void test_ig_root_empty_string(void)
{
    setup();
    char *tmp = make_tmpdir(0); /* no .ig/ → will fallback to cwd */
    if (chdir(tmp) != 0) { perror("chdir"); exit(1); }
    setenv("IG_ROOT", "", 1);

    char cwd_expect[IG_MAX_PATH];
    if (!getcwd(cwd_expect, sizeof(cwd_expect))) { perror("getcwd"); exit(1); }

    char buf[IG_MAX_PATH];
    int rc = resolve_root(buf);
    CHECK(rc == 0);
    /* Empty IG_ROOT is treated as "not set"; should walk-up and fall back */
    CHECK(strcmp(buf, cwd_expect) == 0);

    rmdir_tree(tmp);
    free(tmp);
    teardown();
}

/* 10. cwd est `/` (racine système) → fallback cwd = "/" */
static void test_cwd_is_root(void)
{
    setup();
    if (chdir("/") != 0) { perror("chdir /"); exit(1); }

    char buf[IG_MAX_PATH];
    int rc = resolve_root(buf);
    /* Walk-up from "/" finds no .ig/ and stops; fallback is "/" */
    CHECK(rc == 0);
    CHECK(strcmp(buf, "/") == 0);

    teardown();
}

/* 11. IG_ROOT défini, .ig/ aussi dans cwd → IG_ROOT a la priorité */
static void test_ig_root_priority_over_ig_dir(void)
{
    setup();
    char *tmp_root = make_tmpdir(0);
    char *tmp_cwd  = make_tmpdir(1); /* .ig/ present */
    if (chdir(tmp_cwd) != 0) { perror("chdir tmp_cwd"); exit(1); }
    setenv("IG_ROOT", tmp_root, 1);

    char buf[IG_MAX_PATH];
    int rc = resolve_root(buf);
    CHECK(rc == 0);
    CHECK(strcmp(buf, tmp_root) == 0);

    rmdir_tree(tmp_root);
    free(tmp_root);
    rmdir_tree(tmp_cwd);
    free(tmp_cwd);
    teardown();
}

/* 12. buf rempli correctement — longueur cohérente avec strlen */
static void test_buf_nul_terminated(void)
{
    setup();
    char *tmp = make_tmpdir(1);
    if (chdir(tmp) != 0) { perror("chdir"); exit(1); }

    char buf[IG_MAX_PATH];
    memset(buf, 0xAB, sizeof(buf)); /* poison */
    int rc = resolve_root(buf);
    CHECK(rc == 0);
    /* buf must be a valid C string (NUL before end of array) */
    size_t len = strnlen(buf, IG_MAX_PATH);
    CHECK(len < IG_MAX_PATH);
    CHECK(buf[len] == '\0');

    rmdir_tree(tmp);
    free(tmp);
    teardown();
}

/* ── main ──────────────────────────────────────────────────────────────── */

int main(void)
{
    test_ig_root_absolute();
    test_ig_root_relative();
    test_no_env_ig_in_cwd();
    test_no_env_ig_in_parent();
    test_no_env_ig_five_levels_up();
    test_cap_32_levels();
    test_fallback_to_cwd();
    test_ig_root_nonexistent();
    test_ig_root_empty_string();
    test_cwd_is_root();
    test_ig_root_priority_over_ig_dir();
    test_buf_nul_terminated();

    printf("\n%d/%d tests passed\n", tests_passed, tests_run);
    return (tests_passed == tests_run) ? 0 : 1;
}
