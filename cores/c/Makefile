# Makefile - NES C core. Primary build is nmake + MSVC (vcvarsall x64).
# A GNU make fallback is provided for POSIX systems.
#
# Targets:
#   all       - build static lib nes_core_c.lib
#   test      - build and run test_cpu.exe + test_ppu.exe + test_apu.exe
#   test_cpu  - build and run test_cpu.exe
#   test_ppu  - build and run test_ppu.exe
#   test_apu  - build and run test_apu.exe
#   clean     - remove build artifacts

# MSVC defaults (nmake). Override by setting CC / AR on the command line.
CC   = cl
AR   = lib
LINK = link
CFLAGS = /std:c11 /O2 /W3 /nologo /Iinclude
ARFLAGS = /nologo
LDFLAGS = /nologo

# Core library sources (compiled into nes_core_c.lib).
CORE_SRCS = src\addressing.c src\apu.c src\bus.c src\cartridge.c src\cpu.c src\cpu_unofficial.c src\ppu.c src\ppu_render.c src\region.c
CORE_OBJS = $(CORE_SRCS:.c=.obj)

# Test executables.
TEST_CPU_SRC = tests\test_cpu.c
TEST_CPU_OBJ = $(TEST_CPU_SRC:.c=.obj)
TEST_CPU_EXE = test_cpu.exe

TEST_PPU_SRC = tests\test_ppu.c
TEST_PPU_OBJ = $(TEST_PPU_SRC:.c=.obj)
TEST_PPU_EXE = test_ppu.exe

TEST_APU_SRC = tests\test_apu.c
TEST_APU_OBJ = $(TEST_APU_SRC:.c=.obj)
TEST_APU_EXE = test_apu.exe

# Inference rule: .c -> .obj
.c.obj:
	$(CC) $(CFLAGS) /c $< /Fo:$@

# ---- all: static library ----
all: nes_core_c.lib

nes_core_c.lib: $(CORE_OBJS)
	$(AR) $(ARFLAGS) /out:$@ $(CORE_OBJS)

# ---- test: build and run all test exes ----
test: $(TEST_CPU_EXE) $(TEST_PPU_EXE) $(TEST_APU_EXE)
	$(TEST_CPU_EXE)
	$(TEST_PPU_EXE)
	$(TEST_APU_EXE)

# ---- test_cpu: build and run test_cpu.exe ----
test_cpu: $(TEST_CPU_EXE)
	$(TEST_CPU_EXE)

$(TEST_CPU_EXE): $(TEST_CPU_OBJ) nes_core_c.lib
	$(LINK) $(LDFLAGS) /out:$@ $(TEST_CPU_OBJ) nes_core_c.lib

# ---- test_ppu: build and run test_ppu.exe ----
test_ppu: $(TEST_PPU_EXE)
	$(TEST_PPU_EXE)

$(TEST_PPU_EXE): $(TEST_PPU_OBJ) nes_core_c.lib
	$(LINK) $(LDFLAGS) /out:$@ $(TEST_PPU_OBJ) nes_core_c.lib

# ---- test_apu: build and run test_apu.exe ----
test_apu: $(TEST_APU_EXE)
	$(TEST_APU_EXE)

$(TEST_APU_EXE): $(TEST_APU_OBJ) nes_core_c.lib
	$(LINK) $(LDFLAGS) /out:$@ $(TEST_APU_OBJ) nes_core_c.lib

# ---- clean ----
clean:
	-del /q $(CORE_OBJS) $(TEST_CPU_OBJ) $(TEST_PPU_OBJ) $(TEST_APU_OBJ) nes_core_c.lib $(TEST_CPU_EXE) $(TEST_PPU_EXE) $(TEST_APU_EXE) 2>nul
