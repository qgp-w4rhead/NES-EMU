# cores/java/Makefile — build nes_core_java.dll via GraalVM Native Image.
#
# Prerequisites:
#   - GraalVM CE 21+ installed (GRAALVM_HOME env var or default path below)
#   - MSVC Build Tools 2022 (vcvars64.bat on PATH or at default path below)
#
# Windows (nmake):
#   nmake /f Makefile            # build nes_core_java.dll
#   nmake /f Makefile test       # run JVM tests
#   nmake /f Makefile clean
#
# The build is a two-pass native-image process:
#   1. First pass: native-image --shared (generates nes_core_java.h + graal_isolate.h)
#   2. Compile the C shim (nes_core_java_shim.c) against the generated headers
#   3. Second pass: native-image --shared -H:NativeLinkerOption=shim.obj
#      (links the shim into the DLL, exporting the 13 nes_core_* symbols)

GRAALVM_HOME = C:\Users\W4RHEAD\graalvm-install\graalvm-community-openjdk-21.0.2+13.1
VCVARS = C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat

JAVAC = "$(GRAALVM_HOME)\bin\javac.exe"
JAVA = "$(GRAALVM_HOME)\bin\java.exe"
NATIVE_IMAGE = "$(GRAALVM_HOME)\bin\native-image.cmd"

SRC_DIR = src\main\java
TEST_DIR = test
CLASSES = classes
TEST_CLASSES = test-classes
DLL = nes_core_java.dll
SHIM = nes_core_java_shim.obj

NI_FLAGS = --shared --gc=serial -O3 --no-fallback -march=native
JAVAC_FLAGS = --add-modules org.graalvm.nativeimage,org.graalvm.word

.PHONY: all clean test

all: $(DLL)

# Step 1: compile Java sources
$(CLASSES):
	@if not exist $(CLASSES) mkdir $(CLASSES)
	@for /R $(SRC_DIR) %%f in (*.java) do @echo compiling %%f
	$(JAVAC) $(JAVAC_FLAGS) -d $(CLASSES) $(SRC_DIR)\nes\core\*.java $(SRC_DIR)\nes\core\mappers\*.java

# Step 2: first native-image pass (generates headers)
nes_core_java.h: $(CLASSES)
	cmd /c ""$(VCVARS)" >nul 2>&1 && $(NATIVE_IMAGE) $(NI_FLAGS) -cp $(CLASSES) nes.core.CoreApi -o nes_core_java"

# Step 3: compile C shim
$(SHIM): nes_core_java.h
	cmd /c ""$(VCVARS)" >nul 2>&1 && cl /nologo /c /I . nes_core_java_shim.c /Fo$(SHIM)"

# Step 4: final native-image pass with shim linked in
$(DLL): $(SHIM)
	cmd /c ""$(VCVARS)" >nul 2>&1 && $(NATIVE_IMAGE) $(NI_FLAGS) -cp $(CLASSES) nes.core.CoreApi -o nes_core_java -H:NativeLinkerOption=$(SHIM)"

# Run JVM tests
test: $(CLASSES)
	@if not exist $(TEST_CLASSES) mkdir $(TEST_CLASSES)
	$(JAVAC) -d $(TEST_CLASSES) -cp $(CLASSES) $(TEST_DIR)\nes\core\TestRunner.java
	$(JAVA) -cp "$(CLASSES);$(TEST_CLASSES)" nes.core.TestRunner

clean:
	@if exist $(CLASSES) rmdir /s /q $(CLASSES)
	@if exist $(TEST_CLASSES) rmdir /s /q $(TEST_CLASSES)
	@if exist $(SHIM) del $(SHIM)
	@if exist $(DLL) del $(DLL)
	@if exist nes_core_java.h del nes_core_java.h
	@if exist nes_core_java.lib del nes_core_java.lib
	@if exist graal_isolate.h del graal_isolate.h
	@if exist nes_core_java_dynamic.h del nes_core_java_dynamic.h
	@if exist graal_isolate_dynamic.h del graal_isolate_dynamic.h
