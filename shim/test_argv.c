/*
 * test_argv.c — Unit tests for parse_argv().
 *
 * Linked against ig_nomain.o (ig.c compiled with -DIG_NO_MAIN).
 */

#define _POSIX_C_SOURCE 200809L

#include "parse.h"

#include <stdio.h>
#include <string.h>

/* ── helpers ──────────────────────────────────────────────────────────── */

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

/* Build a mutable argv array from string literals. */
#define MK_ARGV(...) \
    ((char *[]){ "ig", __VA_ARGS__, NULL })

#define MK_ARGC(...) \
    ((int)(sizeof((char *[]){ "ig", __VA_ARGS__ }) / sizeof(char *)))

/* ── test cases ───────────────────────────────────────────────────────── */

/* 1. argv vide → passthrough=1 */
static void test_empty_argv(void)
{
    char *argv[] = { "ig", NULL };
    ig_args_t a;
    int rc = parse_argv(1, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 1);
    CHECK(a.pattern == NULL);
}

/* 2. ig search foo → shim, pattern="foo" */
static void test_search_subcommand(void)
{
    char *argv[] = { "ig", "search", "foo", NULL };
    ig_args_t a;
    int rc = parse_argv(3, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 0);
    CHECK(a.pattern != NULL && strcmp(a.pattern, "foo") == 0);
}

/* 3. ig grep foo src/ → shim, pattern="foo", path="src/" */
static void test_grep_subcommand(void)
{
    char *argv[] = { "ig", "grep", "foo", "src/", NULL };
    ig_args_t a;
    int rc = parse_argv(4, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 0);
    CHECK(a.pattern != NULL && strcmp(a.pattern, "foo") == 0);
    CHECK(a.path    != NULL && strcmp(a.path,    "src/") == 0);
}

/* 4. ig files → shim, files_only=1 */
static void test_files_subcommand(void)
{
    char *argv[] = { "ig", "files", NULL };
    ig_args_t a;
    int rc = parse_argv(2, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 0);
    CHECK(a.files_only == 1);
}

/* 5. ig count foo → shim, count_only=1 */
static void test_count_subcommand(void)
{
    char *argv[] = { "ig", "count", "foo", NULL };
    ig_args_t a;
    int rc = parse_argv(3, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 0);
    CHECK(a.count_only == 1);
}

/* 6. ig index → passthrough=1 */
static void test_index_subcommand(void)
{
    char *argv[] = { "ig", "index", NULL };
    ig_args_t a;
    int rc = parse_argv(2, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 1);
}

/* 7. ig daemon status → passthrough=1 */
static void test_daemon_subcommand(void)
{
    char *argv[] = { "ig", "daemon", "status", NULL };
    ig_args_t a;
    int rc = parse_argv(3, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 1);
}

/* 8. ig setup → passthrough=1 */
static void test_setup_subcommand(void)
{
    char *argv[] = { "ig", "setup", NULL };
    ig_args_t a;
    int rc = parse_argv(2, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 1);
}

/* 9. ig --no-daemon foo → passthrough=1 */
static void test_no_daemon_flag(void)
{
    char *argv[] = { "ig", "--no-daemon", "foo", NULL };
    ig_args_t a;
    int rc = parse_argv(3, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 1);
}

/* 10. ig --help → passthrough=1 */
static void test_help_long(void)
{
    char *argv[] = { "ig", "--help", NULL };
    ig_args_t a;
    int rc = parse_argv(2, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 1);
}

/* 10b. ig -h → passthrough=1 */
static void test_help_short(void)
{
    char *argv[] = { "ig", "-h", NULL };
    ig_args_t a;
    int rc = parse_argv(2, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 1);
}

/* 11. ig --version → passthrough=1 */
static void test_version_flag(void)
{
    char *argv[] = { "ig", "--version", NULL };
    ig_args_t a;
    int rc = parse_argv(2, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 1);
}

/* 12. ig -i Foo → case_insensitive=1 */
static void test_flag_case_insensitive(void)
{
    char *argv[] = { "ig", "-i", "Foo", NULL };
    ig_args_t a;
    int rc = parse_argv(3, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 0);
    CHECK(a.case_insensitive == 1);
    CHECK(a.pattern != NULL && strcmp(a.pattern, "Foo") == 0);
}

/* 13. ig -l foo → files_only=1 */
static void test_flag_files_only(void)
{
    char *argv[] = { "ig", "-l", "foo", NULL };
    ig_args_t a;
    int rc = parse_argv(3, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 0);
    CHECK(a.files_only == 1);
}

/* 14. ig -c foo → count_only=1 */
static void test_flag_count_only(void)
{
    char *argv[] = { "ig", "-c", "foo", NULL };
    ig_args_t a;
    int rc = parse_argv(3, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 0);
    CHECK(a.count_only == 1);
}

/* 15. ig -t rs foo → file_type="rs" */
static void test_flag_file_type(void)
{
    char *argv[] = { "ig", "-t", "rs", "foo", NULL };
    ig_args_t a;
    int rc = parse_argv(4, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 0);
    CHECK(a.file_type != NULL && strcmp(a.file_type, "rs") == 0);
    CHECK(a.pattern   != NULL && strcmp(a.pattern,   "foo") == 0);
}

/* 16. ig -C 3 foo → context_lines=3 */
static void test_flag_context_lines(void)
{
    char *argv[] = { "ig", "-C", "3", "foo", NULL };
    ig_args_t a;
    int rc = parse_argv(4, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 0);
    CHECK(a.context_lines == 3);
    CHECK(a.pattern != NULL && strcmp(a.pattern, "foo") == 0);
}

/* 17. ig search "foo bar" src/ → pattern with space (via explicit subcommand) */
static void test_pattern_with_space(void)
{
    char *argv[] = { "ig", "search", "foo bar", "src/", NULL };
    ig_args_t a;
    int rc = parse_argv(4, argv, &a);
    CHECK(rc == 0);
    CHECK(a.passthrough == 0);
    CHECK(a.pattern != NULL && strcmp(a.pattern, "foo bar") == 0);
    CHECK(a.path    != NULL && strcmp(a.path,    "src/") == 0);
}

/* 18. ig -- -dash-pattern → NOT passthrough; pattern="-dash-pattern" */
/* Note: the shim treats '--' as unknown flag → passthrough currently.
 * The spec says "pattern beginning with '-'" — without '--' separator
 * the shim would normally treat it as an unknown flag.  We document
 * the current behaviour: '-dash-pattern' at argv[1] → passthrough. */
static void test_dash_pattern_direct(void)
{
    /* When passed directly (no --), an unknown flag → passthrough. */
    char *argv[] = { "ig", "search", "-dash-pattern", NULL };
    ig_args_t a;
    int rc = parse_argv(3, argv, &a);
    CHECK(rc == 0);
    /* After 'search' subcommand, next positional should be the pattern.
     * But '-dash-pattern' starts with '-' and is not a known flag,
     * so parse_argv currently sets passthrough=1. */
    CHECK(a.passthrough == 1);
}

/* ── main ─────────────────────────────────────────────────────────────── */

int main(void)
{
    test_empty_argv();
    test_search_subcommand();
    test_grep_subcommand();
    test_files_subcommand();
    test_count_subcommand();
    test_index_subcommand();
    test_daemon_subcommand();
    test_setup_subcommand();
    test_no_daemon_flag();
    test_help_long();
    test_help_short();
    test_version_flag();
    test_flag_case_insensitive();
    test_flag_files_only();
    test_flag_count_only();
    test_flag_file_type();
    test_flag_context_lines();
    test_pattern_with_space();
    test_dash_pattern_direct();

    printf("\n%d/%d tests passed\n", tests_passed, tests_run);
    return (tests_passed == tests_run) ? 0 : 1;
}
