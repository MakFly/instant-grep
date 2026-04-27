#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "parse.h"

/* djb2: seed=5381, h = h*33 + byte (wrapping uint64) — mirrors daemon.rs:socket_path() */
static unsigned long long djb2_hash(const char *s) {
    unsigned long long h = 5381ULL;
    for (const unsigned char *p = (const unsigned char *)s; *p; p++)
        h = h * 33ULL + *p;
    return h;
}

typedef struct { const char *path; unsigned long long expected; } tc_t;

static const tc_t tests[] = {
    { "/",                                              0x000000000002b5d4ULL },
    { "/tmp",                                           0x000000017c78d145ULL },
    { "/Users/x",                                       0x001ae4e07e84e98dULL },
    { "/a/very/long/nested/path",                       0xa979c1ff50756577ULL },
    { "/home/user/.config",                             0x0cb4ce8e6401215eULL },
    { "/var/log/syslog",                                0xbf33a288ba2c255eULL },
    { "/usr/local/bin/ig",                              0x66a9d3b536bd896fULL },
    { "/etc/hosts",                                     0x7267b37a308c4ef0ULL },
    { "/opt/homebrew/bin",                              0x2fbdda03a37d7db7ULL },
    { "/Users/kev/Documents/lab/sandbox/instant-grep", 0x0872125f3406a063ULL },
};

int main(void) {
    int passed = 0;
    int total  = (int)(sizeof tests / sizeof tests[0]);

    for (int i = 0; i < total; i++) {
        unsigned long long got = djb2_hash(tests[i].path);
        if (got == tests[i].expected) {
            printf("PASS [%2d] %-48s -> 0x%016llx\n", i + 1, tests[i].path, got);
            passed++;
        } else {
            printf("FAIL [%2d] %-48s\n"
                   "          expected 0x%016llx\n"
                   "          got      0x%016llx\n",
                   i + 1, tests[i].path,
                   tests[i].expected, got);
        }
    }

    printf("\n%d/%d tests passed\n", passed, total);
    assert(passed == total);
    return passed == total ? 0 : 1;
}
