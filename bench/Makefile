# bench/Makefile — build the cross-language benchmark harness (scaffold).
#
# Full targets land in M3. For now this verifies the harness compiles under a
# C compiler against cores/protocol.h.

CC      ?= cc
CFLAGS  ?= -std=c99 -Wall -Wextra -O2 -I.
LDFLAGS ?=

HARNESS_SRC = harness.c
GENROM_SRC  = gen_rom.c

.PHONY: all clean check

all: harness

harness: $(HARNESS_SRC) harness.h ../cores/protocol.h
	$(CC) $(CFLAGS) -o $@ $(HARNESS_SRC) $(LDFLAGS)

gen_rom: $(GENROM_SRC) ../cores/protocol.h
	$(CC) $(CFLAGS) -c -o $@.o $(GENROM_SRC)

# Compile-only check used by M1 acceptance: header must parse under C.
check: harness.h ../cores/protocol.h
	$(CC) $(CFLAGS) -fsyntax-only $(HARNESS_SRC)

clean:
	rm -f harness gen_rom.o
