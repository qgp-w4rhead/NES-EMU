//
//core_api.zig - NES polyglot core C ABI shim (cores/protocol.h).
//
//Implements the 13-function C ABI defined in cores/protocol.h so the
//benchmark harness in bench/ can load the Zig core as a shared library
//(nes_core_zig.dll) and drive it identically to the Rust / C / C++ cores.
//
//The opaque nes_core_t handle is a heap-allocated EmulatorState (defined
//in the C core's emulator.h). This Zig shim @cImports the C headers to
//get the struct types and function declarations, then calls the C core
//functions directly -- the C core is compiled alongside this Zig code
//via build.zig's addCSourceFiles.
//
//NULL-core checks return 0/NULL/-1 as documented in protocol.h.
//
//M6 key differentiator: the comptime_dispatch module provides a
//comptime 256-entry opcode dispatch table. See comptime_dispatch.zig.
//
const std = @import("std");

// @cImport the C headers for type definitions and function declarations.
// This gives us the EmulatorState, Cartridge, Region, etc. types and all
// the C core function declarations without manual FFI boilerplate.
const c = @cImport({
    @cInclude("protocol.h");
    @cInclude("emulator.h");
    @cInclude("save_state.h");
    @cInclude("cartridge.h");
    @cInclude("region.h");
});

// Import the comptime dispatch module (M6 differentiator).
pub const comptime_dispatch = @import("comptime_dispatch.zig");

// Static implementation identification strings.
const IMPL_NAME: [*:0]const u8 = "zig-nes";
const IMPL_VERSION: [*:0]const u8 = "0.1.0";

// ---- Lifecycle -------------------------------------------------------

export fn nes_core_create(rom_data: [*c]const u8, rom_len: usize) callconv(.C) ?*c.nes_core_t {
    if (rom_data == null or rom_len == 0) {
        return null;
    }

    // Allocate and parse the cartridge.
    const cart = @as([*c]c.Cartridge, @ptrCast(@alignCast(std.c.malloc(@sizeOf(c.Cartridge)))));
    if (cart == null) {
        return null;
    }

    if (c.cartridge_from_bytes(rom_data, rom_len, cart) != 0) {
        std.c.free(cart);
        return null;
    }

    // Allocate and initialize the emulator state.
    const emu = @as([*c]c.EmulatorState, @ptrCast(@alignCast(std.c.malloc(@sizeOf(c.EmulatorState)))));
    if (emu == null) {
        c.cartridge_destroy(cart);
        std.c.free(cart);
        return null;
    }

    c.emulator_init(emu);
    emu.*.cartridge = cart;
    _ = c.bus_insert_cartridge(&emu.*.bus, cart);

    return @ptrCast(@alignCast(emu));
}

export fn nes_core_destroy(core: ?*c.nes_core_t) callconv(.C) void {
    if (core == null) {
        return;
    }
    const emu: *c.EmulatorState = @ptrCast(@alignCast(core));
    c.emulator_destroy(emu);
    std.c.free(emu);
}

export fn nes_core_reset(core: ?*c.nes_core_t) callconv(.C) void {
    if (core == null) {
        return;
    }
    const emu: *c.EmulatorState = @ptrCast(@alignCast(core));
    c.emulator_reset(emu);
}

export fn nes_core_set_region(core: ?*c.nes_core_t, region: c_int) callconv(.C) c_int {
    if (core == null) {
        return -1;
    }
    if (region < 0 or region > @as(c_int, @intCast(c.NES_REGION_DENDY))) {
        return -1;
    }
    const emu: *c.EmulatorState = @ptrCast(@alignCast(core));
    const prev: c_int = @intCast(c.emulator_region(emu));
    c.emulator_set_region(emu, @intCast(region));
    return prev;
}

// ---- Stepping --------------------------------------------------------

export fn nes_core_step_frame(core: ?*c.nes_core_t) callconv(.C) u32 {
    if (core == null) {
        return 0;
    }
    const emu: *c.EmulatorState = @ptrCast(@alignCast(core));
    return c.emulator_step_frame(emu);
}

export fn nes_core_step_instruction(core: ?*c.nes_core_t) callconv(.C) u32 {
    if (core == null) {
        return 0;
    }
    const emu: *c.EmulatorState = @ptrCast(@alignCast(core));
    return c.emulator_step_instruction(emu);
}

// ---- Output ----------------------------------------------------------

export fn nes_core_framebuffer(core: ?*c.nes_core_t) callconv(.C) [*c]const u32 {
    if (core == null) {
        return null;
    }
    const emu: *const c.EmulatorState = @ptrCast(@alignCast(core));
    return c.emulator_framebuffer(emu);
}

export fn nes_core_take_audio(core: ?*c.nes_core_t, buf: [*c]i16, cap: usize) callconv(.C) usize {
    if (core == null or buf == null or cap == 0) {
        return 0;
    }
    const emu: *c.EmulatorState = @ptrCast(@alignCast(core));

    // Allocate a temporary float buffer for the audio samples.
    const tmp = @as([*c]f32, @ptrCast(@alignCast(std.c.malloc(cap * @sizeOf(f32)))));
    if (tmp == null) {
        return 0;
    }
    defer std.c.free(tmp);

    const n = c.emulator_take_audio_samples(emu, tmp, cap);

    // Convert float [-1.0, 1.0] to int16 with saturation.
    var i: usize = 0;
    while (i < n) : (i += 1) {
        var s: f32 = tmp[i];
        if (s > 1.0) s = 1.0;
        if (s < -1.0) s = -1.0;
        const v: i32 = if (s >= 0.0) blk: {
            const r: i32 = @intFromFloat(s * 32767.0);
            break :blk if (r > 32767) 32767 else r;
        } else blk: {
            const r: i32 = @intFromFloat(s * 32768.0);
            break :blk if (r < -32768) -32768 else r;
        };
        buf[i] = @intCast(v);
    }
    return n;
}

// ---- Save state ------------------------------------------------------

export fn nes_core_save_state(core: ?*c.nes_core_t, buf: [*c]u8, cap: usize) callconv(.C) usize {
    // Match the Rust reference: a NULL/zero-cap buffer is an error.
    if (core == null or buf == null or cap == 0) {
        return 0;
    }
    const emu: *const c.EmulatorState = @ptrCast(@alignCast(core));
    return c.emulator_save_state(emu, buf, cap);
}

export fn nes_core_load_state(core: ?*c.nes_core_t, buf: [*c]const u8, len: usize) callconv(.C) c_int {
    if (core == null or buf == null or len == 0) {
        return 0;
    }
    const emu: *c.EmulatorState = @ptrCast(@alignCast(core));
    return if (c.emulator_load_state(emu, buf, len)) 1 else 0;
}

// ---- Introspection ---------------------------------------------------

export fn nes_core_mapper_number(core: ?*c.nes_core_t) callconv(.C) u16 {
    if (core == null) {
        return 0;
    }
    const emu: *const c.EmulatorState = @ptrCast(@alignCast(core));
    return c.emulator_mapper_number(emu);
}

export fn nes_core_impl_name() callconv(.C) [*c]const u8 {
    return @ptrCast(IMPL_NAME);
}

export fn nes_core_impl_version() callconv(.C) [*c]const u8 {
    return @ptrCast(IMPL_VERSION);
}

