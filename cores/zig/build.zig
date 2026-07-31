//
//build.zig - NES Zig core build system (M6).
//
//Compiles the C core source files (cores/c/src/*.c, except core_api.c)
//via Zig's built-in C compiler and links them with Zig source files
//(core_api.zig + comptime_dispatch.zig) into nes_core_zig.dll.
//
//The Zig core_api.zig provides the 13 protocol.h C ABI exports,
//replacing the C core_api.c. The comptime_dispatch.zig provides a
//comptime 256-entry opcode dispatch table -- the key M6 differentiator.
//
//Build:  zig build --release=fast
//Output: zig-out/bin/nes_core_zig.dll
//
const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{
        .preferred_optimize_mode = .ReleaseFast,
    });

    // Shared library (DLL) exporting the 13 protocol.h symbols.
    const lib = b.addSharedLibrary(.{
        .name = "nes_core_zig",
        .root_source_file = b.path("src/core_api.zig"),
        .target = target,
        .optimize = optimize,
    });

    // Include paths for C compilation and Zig @cImport.
    lib.addIncludePath(b.path("../c/include")); // C headers (cpu.h, bus.h, etc.)
    lib.addIncludePath(b.path("..")); // protocol.h (cores/protocol.h)

    // All C source files from cores/c/src/ EXCEPT core_api.c (replaced by Zig).
    const c_sources = [_][]const u8{
        "../c/src/addressing.c",
        "../c/src/apu.c",
        "../c/src/bus.c",
        "../c/src/cartridge.c",
        "../c/src/cpu.c",
        "../c/src/cpu_unofficial.c",
        "../c/src/ppu.c",
        "../c/src/ppu_render.c",
        "../c/src/region.c",
        "../c/src/mappers.c",
        "../c/src/nrom.c",
        "../c/src/mmc1.c",
        "../c/src/mmc2.c",
        "../c/src/mmc3.c",
        "../c/src/mmc5.c",
        "../c/src/uxrom.c",
        "../c/src/cnrom.c",
        "../c/src/axrom.c",
        "../c/src/vrc6.c",
        "../c/src/vrc7.c",
        "../c/src/fme7.c",
        "../c/src/namco163.c",
        "../c/src/fds.c",
        "../c/src/fds_audio.c",
        "../c/src/opll.c",
        "../c/src/ym2149.c",
        "../c/src/joypad.c",
        "../c/src/emulator.c",
        "../c/src/save_state.c",
    };

    lib.addCSourceFiles(.{
        .files = &c_sources,
        .flags = &.{ "-std=c11", "-O2" },
    });

    lib.linkLibC();
    b.installArtifact(lib);

    // ---- Tests for comptime dispatch table ----
    const tests = b.addTest(.{
        .name = "comptime_dispatch_test",
        .root_source_file = b.path("src/comptime_dispatch.zig"),
        .target = target,
        .optimize = optimize,
    });
    tests.addIncludePath(b.path("../c/include"));
    tests.addIncludePath(b.path(".."));
    tests.linkLibC();

    const run_tests = b.addRunArtifact(tests);
    const test_step = b.step("test", "Run comptime dispatch table tests");
    test_step.dependOn(&run_tests.step);
}

