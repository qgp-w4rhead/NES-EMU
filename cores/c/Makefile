# Makefile - NES C core (M4.1). Primary build is nmake + MSVC (vcvarsall x64).
# A GNU make fallback is provided for POSIX systems.
#
# Targets:
#   all   - build static lib nes_core_c.lib
#   test  - build and run test_cpu.exe
#   clean - remove build artifacts

# MSVC defaults (nmake). Override by setting CC / AR on the command line.
CC   = cl
AR   = lib
LINK = link
CFLAGS = /std:c11 /O2 /W3 /nologo /Iinclude
ARFLAGS = /nologo
LDFLAGS = /nologo

# Core library sources (compiled into nes_core_c.lib).
CORE_SRCS = src\addressing.c src\bus.c src\cartridge.c src\cpu.c src\cpu_unofficial.c src\region.c
CORE_OBJS = $(CORE_SRCS:.c=.obj)

# Test executable.
TEST_SRC = tests\test_cpu.c
TEST_OBJ = $(TEST_SRC:.c=.obj)
TEST_EXE = test_cpu.exe

# Inference rule: .c -> .obj
.c.obj:
	$(CC) $(CFLAGS) /c $< /Fo:$@

# ---- all: static library ----
all: nes_core_c.lib

nes_core_c.lib: $(CORE_OBJS)
	$(AR) $(ARFLAGS) /out:$@ $(CORE_OBJS)

# ---- test: build and run test_cpu.exe ----
test: $(TEST_EXE)
	$(TEST_EXE)

$(TEST_EXE): $(TEST_OBJ) nes_core_c.lib
	$(LINK) $(LDFLAGS) /out:$@ $(TEST_OBJ) nes_core_c.lib

# ---- clean ----
clean:
	-del /q $(CORE_OBJS) $(TEST_OBJ) nes_core_c.lib $(TEST_EXE) 2>nul
