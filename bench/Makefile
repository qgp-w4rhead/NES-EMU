# bench/Makefile — build the cross-language benchmark harness.
#
# Builds bench/harness(.exe) which loads any core shared library
# implementing cores/protocol.h and runs the M3 benchmark suite.
#
# Windows (MSVC, via vcvarsall):
#   nmake /f Makefile            # build harness.exe
#   nmake /f Makefile clean
#   (or use ..\build-harness.ps1 which calls vcvarsall for you)
#
# Linux / macOS (GNU make + cc):
#   make                         # build harness
#   make clean
#
# The Makefile auto-detects Windows via the OS environment variable and
# selects cl/nmake-style flags; otherwise it falls back to cc/GNU make.

!IF "$(OS)" == "Windows_NT"
# ---- Windows / MSVC (nmake) ----
CC      = cl
CFLAGS  = /nologo /std:c11 /O2 /W3 /wd4996 /D_CRT_SECURE_NO_WARNINGS /I.
LDFLAGS = /nologo
HARNESS = harness.exe
OBJ     = obj
LINK    = $(CC) /Fe:$(HARNESS) $(LDFLAGS)
RM      = -del 2>NUL
!ELSE
# ---- POSIX (GNU make + cc) ----
CC     ?= cc
CFLAGS ?= -std=c11 -O2 -Wall -Wextra -I.
LDFLAGS ?=
HARNESS = harness
OBJ    = o
LINK   = $(CC) -o $(HARNESS) $(LDFLAGS)
RM     = rm -f
!ENDIF

HEADERS = harness.h gen_rom.h ../cores/protocol.h
OBJS    = harness.$(OBJ) gen_rom.$(OBJ)

.PHONY: all clean check rust-core

all: $(HARNESS)

$(HARNESS): $(OBJS)
	$(LINK) $(OBJS)

harness.$(OBJ): harness.c $(HEADERS)
	$(CC) $(CFLAGS) /c harness.c /Fo:harness.$(OBJ)

gen_rom.$(OBJ): gen_rom.c gen_rom.h ../cores/protocol.h
	$(CC) $(CFLAGS) /c gen_rom.c /Fo:gen_rom.$(OBJ)

# Compile-only check (header parses under C).
check: $(HEADERS)
	$(CC) $(CFLAGS) /c harness.c /Fo:NUL

# Build the reference Rust cdylib (invokes cargo in cores/rust).
rust-core:
	cd ..\cores\rust && cargo build --release

clean:
	$(RM) $(HARNESS) $(OBJS)
