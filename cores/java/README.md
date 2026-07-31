# M9: Java NES Core (GraalVM Native Image)

Full NES emulator core ported from C to Java, AOT-compiled via GraalVM Native
Image into `nes_core_java.dll` exporting the 13 C ABI symbols from
`cores/protocol.h`.

## Architecture

```
nes_core_java.dll
├── Java @CEntryPoint (java_*)     ← GraalVM native-image --shared
│   └── nes.core.CoreApi           ← handle registry + off-heap framebuffer
│       └── nes.core.Emulator      ← frame loop (CPU/PPU/APU/bus)
│           ├── Cpu (6502 + 13 addr modes + 151 official + 105 unofficial ops)
│           ├── Ppu + PpuRender (scanline renderer, NTSC/PAL/Dendy palettes)
│           ├── Apu + ApuChannels (2 pulse, triangle, noise, DMC, frame counter)
│           ├── Bus (address decode, mirroring, OAM-DMA)
│           ├── Cartridge (iNES loader)
│           ├── 13 mappers + 4 expansion audio chips
│           └── SaveState (manual binary serialization)
└── C shim (nes_core_java_shim.c)  ← __declspec(dllexport) nes_core_*
    └── manages global graal_isolate_t, forwards to java_*
```

## FFI Strategy

GraalVM `@CEntryPoint` methods require an `IsolateThread` first parameter,
but `cores/protocol.h` signatures don't include one. The C shim bridges this:

1. Java `@CEntryPoint(name = "java_*")` methods take `IsolateThread` first.
2. C shim exports `nes_core_*` (exact protocol.h signatures) with
   `__declspec(dllexport)`, manages a global graal isolate lazily, and
   forwards to `java_*`.
3. The shim is compiled to `.obj` and linked into the DLL via
   `-H:NativeLinkerOption=shim.obj`.

The opaque `nes_core_t*` handle is a pointer-sized integer key into a static
`HashMap<Integer, Emulator>` registry. The framebuffer is an off-heap
`UnmanagedMemory.malloc` buffer (Word types can't be stored in HashMap under
native-image, so raw `long` addresses are stored and reconstructed via
`WordFactory.pointer(addr)`).

## Build

```powershell
# Prerequisites: GraalVM CE 21+ + MSVC Build Tools 2022
$gv = "C:\Users\W4RHEAD\graalvm-install\graalvm-community-openjdk-21.0.2+13.1"
$vcvars = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"

# 1. Compile Java sources
& "$gv\bin\javac.exe" --add-modules org.graalvm.nativeimage,org.graalvm.word -d classes (Get-ChildItem src -Recurse -Filter *.java | ForEach-Object { $_.FullName })

# 2. First native-image pass (generates nes_core_java.h + graal_isolate.h)
cmd /c "`"$vcvars`" >nul 2>&1 && `"$gv\bin\native-image.cmd`" --shared --gc=serial -O3 --no-fallback -cp classes nes.core.CoreApi -o nes_core_java"

# 3. Compile C shim
cmd /c "`"$vcvars`" >nul 2>&1 && cl /nologo /c /I . nes_core_java_shim.c /Fo nes_core_java_shim.obj"

# 4. Final native-image pass (links shim into DLL)
cmd /c "`"$vcvars`" >nul 2>&1 && `"$gv\bin\native-image.cmd`" --shared --gc=serial -O3 --no-fallback -cp classes nes.core.CoreApi -o nes_core_java -H:NativeLinkerOption=nes_core_java_shim.obj"
```

## Test

```powershell
& "$gv\bin\javac.exe" -d test-classes -cp classes test\nes\core\TestRunner.java
& "$gv\bin\java.exe" -cp "classes;test-classes" nes.core.TestRunner
```

## Benchmark

```powershell
# From repo root:
.\bench\harness.exe cores\java\nes_core_java.dll
.\bench\fb_compare.exe cores\c\nes_core_c.dll cores\java\nes_core_java.dll 20
```

## Performance (vs C core)

| Benchmark    | Java (us) | C (us) | Ratio |
|-------------|-----------|--------|-------|
| step_frame   | 1789      | 2080   | 0.86x (faster) |
| cpu_step     | 0.229     | 0.163  | 1.40x |
| save_state   | 234       | 2.3    | 102x (managed serialization) |
| render_frame | 1789      | 2081   | 0.86x (faster) |

Framebuffer + audio + cycle counts are byte-identical to the C core across
20 frames (0 mismatches via fb_compare).
