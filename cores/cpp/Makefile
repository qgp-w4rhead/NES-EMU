# Makefile - NES C++ core. nmake + MSVC (vcvarsall x64).
#
# Targets:
#   all       - build static lib nes_core_cpp.lib
#   dll       - build nes_core_cpp.dll
#   test      - build and run all test exes
#   clean     - remove build artifacts

CC   = cl
AR   = lib
LINK = link
CXXFLAGS = /std:c++20 /O2 /W3 /nologo /EHsc /Iinclude /I..
ARFLAGS = /nologo
LDFLAGS = /nologo

# Core library sources (compiled into nes_core_cpp.lib).
CORE_SRCS = src\addressing.cpp src\apu.cpp src\bus.cpp src\cartridge.cpp src\cpu.cpp \
            src\cpu_unofficial.cpp src\ppu.cpp src\ppu_render.cpp src\region.cpp \
            src\mappers.cpp src\nrom.cpp src\mmc1.cpp src\mmc2.cpp src\mmc3.cpp \
            src\mmc5.cpp src\uxrom.cpp src\cnrom.cpp src\axrom.cpp src\vrc6.cpp \
            src\vrc7.cpp src\fme7.cpp src\namco163.cpp src\fds.cpp src\fds_audio.cpp \
            src\opll.cpp src\ym2149.cpp src\joypad.cpp src\emulator.cpp \
            src\save_state.cpp
CORE_OBJS = $(CORE_SRCS:.cpp=.obj)

# core_api.cpp is compiled separately (it includes ../protocol.h which lives
# outside the include/ dir) and linked into the DLL only.
CORE_API_SRC = src\core_api.cpp
CORE_API_OBJ = $(CORE_API_SRC:.cpp=.obj)

# Test executables.
TEST_CPU_SRC = tests\test_cpu.cpp
TEST_CPU_OBJ = $(TEST_CPU_SRC:.cpp=.obj)
TEST_CPU_EXE = test_cpu.exe

TEST_PPU_SRC = tests\test_ppu.cpp
TEST_PPU_OBJ = $(TEST_PPU_SRC:.cpp=.obj)
TEST_PPU_EXE = test_ppu.exe

TEST_APU_SRC = tests\test_apu.cpp
TEST_APU_OBJ = $(TEST_APU_SRC:.cpp=.obj)
TEST_APU_EXE = test_apu.exe

TEST_MAP_SRC = tests\test_mappers.cpp
TEST_MAP_OBJ = $(TEST_MAP_SRC:.cpp=.obj)
TEST_MAP_EXE = test_mappers.exe

TEST_SAVE_SRC = tests\test_save.cpp
TEST_SAVE_OBJ = $(TEST_SAVE_SRC:.cpp=.obj)
TEST_SAVE_EXE = test_save.exe

# Inference rule: .cpp -> .obj
.cpp.obj:
	$(CC) $(CXXFLAGS) /c $< /Fo:$@

# ---- all: static library + DLL ----
all: nes_core_cpp.lib nes_core_cpp.dll

nes_core_cpp.lib: $(CORE_OBJS)
	$(AR) $(ARFLAGS) /out:$@ $(CORE_OBJS)

# ---- DLL: nes_core_cpp.dll (links core_api.obj against the static lib) ----
nes_core_cpp.dll: $(CORE_API_OBJ) nes_core_cpp.lib nes_core.def
	$(LINK) /DLL /nologo /out:$@ /DEF:nes_core.def /IMPLIB:nes_core_cpp_dll.lib $(CORE_API_OBJ) nes_core_cpp.lib

# ---- test: build and run all test exes ----
test: $(TEST_CPU_EXE) $(TEST_PPU_EXE) $(TEST_APU_EXE) $(TEST_MAP_EXE) $(TEST_SAVE_EXE)
	$(TEST_CPU_EXE)
	$(TEST_PPU_EXE)
	$(TEST_APU_EXE)
	$(TEST_MAP_EXE)
	$(TEST_SAVE_EXE)

$(TEST_CPU_EXE): $(TEST_CPU_OBJ) nes_core_cpp.lib
	$(LINK) $(LDFLAGS) /out:$@ $(TEST_CPU_OBJ) nes_core_cpp.lib

$(TEST_PPU_EXE): $(TEST_PPU_OBJ) nes_core_cpp.lib
	$(LINK) $(LDFLAGS) /out:$@ $(TEST_PPU_OBJ) nes_core_cpp.lib

$(TEST_APU_EXE): $(TEST_APU_OBJ) nes_core_cpp.lib
	$(LINK) $(LDFLAGS) /out:$@ $(TEST_APU_OBJ) nes_core_cpp.lib

$(TEST_MAP_EXE): $(TEST_MAP_OBJ) nes_core_cpp.lib
	$(LINK) $(LDFLAGS) /out:$@ $(TEST_MAP_OBJ) nes_core_cpp.lib

$(TEST_SAVE_EXE): $(TEST_SAVE_OBJ) nes_core_cpp.lib
	$(LINK) $(LDFLAGS) /out:$@ $(TEST_SAVE_OBJ) nes_core_cpp.lib

# ---- clean ----
clean:
	-del /q $(CORE_OBJS) $(CORE_API_OBJ) $(TEST_CPU_OBJ) $(TEST_PPU_OBJ) $(TEST_APU_OBJ) $(TEST_MAP_OBJ) $(TEST_SAVE_OBJ) nes_core_cpp.lib nes_core_cpp.dll nes_core_cpp.exp nes_core_cpp_dll.lib nes_core_cpp_dll.exp $(TEST_CPU_EXE) $(TEST_PPU_EXE) $(TEST_APU_EXE) $(TEST_MAP_EXE) $(TEST_SAVE_EXE) 2>nul
