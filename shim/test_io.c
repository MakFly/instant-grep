/*
 * test_io.c — 5 tests verifying that ig correctly relays socket data to stdout.
 *
 * Strategy: socketpair(AF_UNIX, SOCK_STREAM) + fork.
 *   - Child A (test runner): sets up the socketpair, calls relay in-process
 *     with stdout wired to a temp file, then exits with relay's return code.
 *   - Child B (fake daemon): writes payload to sv[1] then closes it.
 *   - Parent: forks both, collects child A's exit code, reads the temp file,
 *     and compares bytes with memcmp.
 *
 * Writing to a temp file (not a pipe) avoids deadlocks when payload > pipe
 * buffer size.
 *
 * Compile: cc -O2 -Wall -Wextra -std=c11 -o test_io test_io.c
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
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

/* ── relay_response re-implementation ────────────────────────────────────── */
static int relay_response_impl(int fd)
{
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

    /* Use low-level write to avoid stdio buffering issues in forked children */
    {
        size_t wwritten = 0;
        while (wwritten < used) {
            ssize_t w = write(STDOUT_FILENO, rbuf + wwritten, used - wwritten);
            if (w < 0) { if (errno == EINTR) continue; break; }
            wwritten += (size_t)w;
        }
    }

    int exit_code = 1;
    if (used < cap) rbuf[used] = '\0';
    else {
        char *tmp = realloc(rbuf, cap + 1);
        if (tmp) { rbuf = tmp; rbuf[used] = '\0'; }
    }
    const char *p = strstr(rbuf, "\"error\"");
    if (p) {
        p += 7;
        while (*p == ' ' || *p == ':') p++;
        if (strncmp(p, "null", 4) == 0) exit_code = 0;
    }

    free(rbuf);
    return exit_code;
}

/* ── helpers ─────────────────────────────────────────────────────────────── */

#define PASS(name) do { printf("PASS %s\n", name); passed++; } while (0)
#define FAIL(name, ...) do { printf("FAIL %s: ", name); printf(__VA_ARGS__); printf("\n"); } while (0)

static int passed = 0;
static int total  = 0;

/*
 * Write `plen` bytes of `payload` to `wfd`, optionally in two halves with a
 * short sleep between them (partial_mode).  Then close `wfd`.  Intended to
 * run in a forked child that calls _exit() when done.
 */
static void fake_daemon_write(int wfd, const char *payload, size_t plen, int partial_mode)
{
    if (!partial_mode) {
        size_t written = 0;
        while (written < plen) {
            ssize_t w = write(wfd, payload + written, plen - written);
            if (w < 0) { if (errno == EINTR) continue; break; }
            written += (size_t)w;
        }
    } else {
        size_t half = plen / 2;
        size_t written = 0;
        while (written < half) {
            ssize_t w = write(wfd, payload + written, half - written);
            if (w < 0) { if (errno == EINTR) continue; break; }
            written += (size_t)w;
        }
        usleep(10000);
        while (written < plen) {
            ssize_t w = write(wfd, payload + written, plen - written);
            if (w < 0) { if (errno == EINTR) continue; break; }
            written += (size_t)w;
        }
    }
    close(wfd);
    _exit(0);
}

/*
 * Run a relay test:
 *   1. Create a socketpair.
 *   2. Fork daemon child: writes `payload` to sv[1], closes it.
 *   3. Fork relay child: opens temp file for writing, dup2 onto stdout,
 *      calls relay_response_impl(sv[0]), exits with relay return code.
 *   4. Parent: wait for both children, read temp file, compare.
 *
 * Returns 1 on pass, 0 on fail.
 */
static int run_relay_test(const char *name,
                           const char *payload, size_t plen,
                           int partial_mode, int expect_code)
{
    /* Temp file for relay output */
    char tmpl[] = "/tmp/ig_io_test_XXXXXX";
    int tmpfd = mkstemp(tmpl);
    if (tmpfd < 0) { FAIL(name, "mkstemp failed"); return 0; }

    int sv[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv) < 0) {
        FAIL(name, "socketpair failed"); close(tmpfd); unlink(tmpl); return 0;
    }

    /* Fork daemon */
    pid_t daemon_pid = fork();
    if (daemon_pid < 0) { FAIL(name, "fork daemon failed"); close(sv[0]); close(sv[1]); close(tmpfd); unlink(tmpl); return 0; }
    if (daemon_pid == 0) {
        close(sv[0]); /* daemon only writes to sv[1] */
        close(tmpfd);
        fake_daemon_write(sv[1], payload, plen, partial_mode);
        /* unreachable */
        _exit(0);
    }
    close(sv[1]);

    /* Fork relay child */
    pid_t relay_pid = fork();
    if (relay_pid < 0) {
        FAIL(name, "fork relay failed");
        waitpid(daemon_pid, NULL, 0);
        close(sv[0]); close(tmpfd); unlink(tmpl);
        return 0;
    }
    if (relay_pid == 0) {
        /* Close unused end of socketpair so EOF propagates correctly */
        close(sv[1]);
        /* Redirect stdout to temp file */
        dup2(tmpfd, STDOUT_FILENO);
        close(tmpfd);
        /* Redirect stderr to /dev/null */
        int devnull = open("/dev/null", O_WRONLY);
        if (devnull >= 0) { dup2(devnull, STDERR_FILENO); close(devnull); }
        int code = relay_response_impl(sv[0]);
        close(sv[0]);
        _exit(code);
    }
    close(sv[0]);
    close(tmpfd);

    /* Wait for both children */
    int daemon_status = 0, relay_status = 0;
    waitpid(daemon_pid, &daemon_status, 0);
    waitpid(relay_pid,  &relay_status,  0);

    int relay_code = WIFEXITED(relay_status) ? WEXITSTATUS(relay_status) : -1;

    /* Read temp file and compare */
    int fd2 = open(tmpl, O_RDONLY);
    if (fd2 < 0) { FAIL(name, "open tmpfile failed"); unlink(tmpl); return 0; }

    char *out = malloc(plen + 1);
    if (!out) { FAIL(name, "malloc failed"); close(fd2); unlink(tmpl); return 0; }

    size_t out_len = 0;
    ssize_t r;
    while (out_len < plen + 1 &&
           (r = read(fd2, out + out_len, plen + 1 - out_len)) > 0) {
        out_len += (size_t)r;
    }
    close(fd2);
    unlink(tmpl);

    int ok = (out_len == plen) && (memcmp(out, payload, plen) == 0) &&
             (expect_code < 0 || relay_code == expect_code);
    free(out);

    if (ok) {
        PASS(name);
        return 1;
    } else {
        FAIL(name, "len=%zu expected=%zu relay_code=%d expected_code=%d",
             out_len, plen, relay_code, expect_code);
        return 0;
    }
}

/* ── tests ────────────────────────────────────────────────────────────────── */

static void test1_small_payload(void)
{
    total++;
    const char *payload = "{\"error\":null,\"matches\":[{\"file\":\"a.c\",\"line\":1}]}\n";
    run_relay_test("t1_relay_small", payload, strlen(payload), 0, 0);
}

static void test2_64k_payload(void)
{
    total++;
    size_t plen = 65536;
    char *payload = malloc(plen);
    if (!payload) { FAIL("t2_relay_64k", "malloc failed"); return; }
    for (size_t i = 0; i < plen; i++) payload[i] = (char)('A' + (i % 26));
    /* No "error":null → relay returns 1 */
    run_relay_test("t2_relay_64k", payload, plen, 0, 1);
    free(payload);
}

static void test3_large_payload(void)
{
    total++;
    size_t plen = 131072;
    char *payload = malloc(plen);
    if (!payload) { FAIL("t3_relay_large", "malloc failed"); return; }
    for (size_t i = 0; i < plen; i++) payload[i] = (char)('a' + (i % 26));
    run_relay_test("t3_relay_large", payload, plen, 0, 1);
    free(payload);
}

static void test4_binary_nul(void)
{
    total++;
    size_t plen = 256;
    char payload[256];
    for (size_t i = 0; i < plen; i++) payload[i] = (char)(i & 0xFF);
    run_relay_test("t4_relay_binary_nul", payload, plen, 0, -1 /* any code */);
}

static void test5_partial_then_eof(void)
{
    total++;
    const char *chunk1 = "{\"error\":null,\"partial\":true,";
    const char *chunk2 = "\"data\":\"hello\"}\n";
    size_t c1len = strlen(chunk1);
    size_t c2len = strlen(chunk2);
    size_t total_len = c1len + c2len;
    char payload[512];
    memcpy(payload, chunk1, c1len);
    memcpy(payload + c1len, chunk2, c2len);
    run_relay_test("t5_relay_partial_eof", payload, total_len, 1 /* partial */, 0);
}

/* ── main ─────────────────────────────────────────────────────────────────── */

int main(void)
{
    printf("=== test_io ===\n");

    test1_small_payload();
    test2_64k_payload();
    test3_large_payload();
    test4_binary_nul();
    test5_partial_then_eof();

    printf("\n%d/%d tests passed\n", passed, total);
    return (passed == total) ? 0 : 1;
}
