/*
 * test_json.c — snapshot tests for build_json_request().
 *
 * Linked against ig_nomain.o (ig.c compiled with -DIG_NO_MAIN).
 */

#define _POSIX_C_SOURCE 200809L

#include "parse.h"

#include <assert.h>
#include <stdio.h>
#include <string.h>

/* ── test helpers ────────────────────────────────────────────────────────── */

static int passed = 0;
static int failed = 0;

static void check(int num, const char *label, const char *expected, const ig_args_t *args)
{
    char buf[8192];
    int  ret = build_json_request(args, buf, sizeof(buf));

    if (ret < 0) {
        printf("FAIL [%2d] %s\n         build_json_request returned -1\n", num, label);
        failed++;
        return;
    }

    if (strcmp(buf, expected) == 0) {
        printf("PASS [%2d] %s\n", num, label);
        passed++;
    } else {
        printf("FAIL [%2d] %s\n"
               "         expected: %s\n"
               "         got:      %s\n",
               num, label, expected, buf);
        failed++;
    }
}

/* ── tests ───────────────────────────────────────────────────────────────── */

int main(void)
{
    ig_args_t a;

    /* 1: pattern simple */
    memset(&a, 0, sizeof(a));
    a.pattern = "foo";
    check(1, "pattern simple",
        "{\"pattern\":\"foo\",\"case_insensitive\":false,\"files_only\":false,\"count_only\":false,\"context\":0}",
        &a);

    /* 2: pattern avec guillemets (escape \") */
    memset(&a, 0, sizeof(a));
    a.pattern = "say \"hello\"";
    check(2, "pattern avec guillemets",
        "{\"pattern\":\"say \\\"hello\\\"\",\"case_insensitive\":false,\"files_only\":false,\"count_only\":false,\"context\":0}",
        &a);

    /* 3: pattern avec backslash (escape \\) */
    memset(&a, 0, sizeof(a));
    a.pattern = "C:\\path";
    check(3, "pattern avec backslash",
        "{\"pattern\":\"C:\\\\path\",\"case_insensitive\":false,\"files_only\":false,\"count_only\":false,\"context\":0}",
        &a);

    /* 4: pattern avec \n et \t */
    memset(&a, 0, sizeof(a));
    a.pattern = "foo\nbar\tbaz";
    check(4, "pattern avec newline et tab",
        "{\"pattern\":\"foo\\nbar\\tbaz\",\"case_insensitive\":false,\"files_only\":false,\"count_only\":false,\"context\":0}",
        &a);

    /* 5: pattern UTF-8 multi-byte (passthrough) */
    memset(&a, 0, sizeof(a));
    a.pattern = "caf\xc3\xa9 r\xc3\xa9sum\xc3\xa9";
    check(5, "pattern UTF-8 multi-byte passthrough",
        "{\"pattern\":\"caf\xc3\xa9 r\xc3\xa9sum\xc3\xa9\",\"case_insensitive\":false,\"files_only\":false,\"count_only\":false,\"context\":0}",
        &a);

    /* 6: case_insensitive=true */
    memset(&a, 0, sizeof(a));
    a.pattern = "foo";
    a.case_insensitive = 1;
    check(6, "case_insensitive=true",
        "{\"pattern\":\"foo\",\"case_insensitive\":true,\"files_only\":false,\"count_only\":false,\"context\":0}",
        &a);

    /* 7: files_only=true */
    memset(&a, 0, sizeof(a));
    a.pattern = "foo";
    a.files_only = 1;
    check(7, "files_only=true",
        "{\"pattern\":\"foo\",\"case_insensitive\":false,\"files_only\":true,\"count_only\":false,\"context\":0}",
        &a);

    /* 8: count_only=true */
    memset(&a, 0, sizeof(a));
    a.pattern = "foo";
    a.count_only = 1;
    check(8, "count_only=true",
        "{\"pattern\":\"foo\",\"case_insensitive\":false,\"files_only\":false,\"count_only\":true,\"context\":0}",
        &a);

    /* 9: context=5 */
    memset(&a, 0, sizeof(a));
    a.pattern = "foo";
    a.context_lines = 5;
    check(9, "context=5",
        "{\"pattern\":\"foo\",\"case_insensitive\":false,\"files_only\":false,\"count_only\":false,\"context\":5}",
        &a);

    /* 10: file_type="rs" */
    memset(&a, 0, sizeof(a));
    a.pattern = "foo";
    a.file_type = "rs";
    check(10, "file_type=\"rs\"",
        "{\"pattern\":\"foo\",\"case_insensitive\":false,\"files_only\":false,\"count_only\":false,\"context\":0,\"type\":\"rs\"}",
        &a);

    /* 11: file_type=NULL (champ absent) */
    memset(&a, 0, sizeof(a));
    a.pattern = "foo";
    a.file_type = NULL;
    check(11, "file_type=NULL (absent)",
        "{\"pattern\":\"foo\",\"case_insensitive\":false,\"files_only\":false,\"count_only\":false,\"context\":0}",
        &a);

    /* 12: combinaison de tous les flags */
    memset(&a, 0, sizeof(a));
    a.pattern          = "hello \"world\"";
    a.case_insensitive = 1;
    a.files_only       = 1;
    a.count_only       = 1;
    a.context_lines    = 3;
    a.file_type        = "ts";
    check(12, "combinaison de tous les flags",
        "{\"pattern\":\"hello \\\"world\\\"\",\"case_insensitive\":true,\"files_only\":true,\"count_only\":true,\"context\":3,\"type\":\"ts\"}",
        &a);

    printf("\n%d/%d tests passed\n", passed, passed + failed);
    assert(failed == 0);
    return failed == 0 ? 0 : 1;
}
