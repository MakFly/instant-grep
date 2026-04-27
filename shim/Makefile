CC      ?= cc
CFLAGS  ?= -O2 -Wall -Wextra -std=c11

# All test sources present in the directory
TEST_SRCS := $(wildcard test_*.c)
TEST_BINS := $(TEST_SRCS:.c=)

.PHONY: all ig clean install test

all: ig

ig: ig.c parse.h
	$(CC) $(CFLAGS) -o ig ig.c

ig_nomain.o: ig.c parse.h
	$(CC) $(CFLAGS) -DIG_NO_MAIN -c ig.c -o ig_nomain.o

# Build any test binary from its .c file (link against ig_nomain.o for access to parse functions)
test_%: test_%.c ig_nomain.o parse.h
	$(CC) $(CFLAGS) -o $@ $< ig_nomain.o

# Compile and run all test_*.c that exist; use - prefix so a missing source is not fatal
test: ig $(TEST_BINS)
	@failed=0; \
	for t in $(TEST_BINS); do \
		echo "=== $$t ==="; \
		./$$t || failed=$$((failed + 1)); \
	done; \
	if [ $$failed -ne 0 ]; then echo "$$failed test suite(s) FAILED"; exit 1; fi; \
	echo "All test suites passed."

clean:
	rm -f ig ig_nomain.o $(TEST_BINS)

install: ig
	cp ig $(DESTDIR)/usr/local/bin/ig
