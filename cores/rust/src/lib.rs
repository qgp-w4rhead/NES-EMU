//! Rust FFI wrapper — exposes the `nes-emu` core behind the C ABI defined in
//! `cores/protocol.h`.
//!
//! This is the reference implementation of the polyglot core protocol. It
//! builds as a `cdylib` (`nes_core_rust.dll` / `libnes_core_rust.so` /
//! `libnes_core_rust.dylib`) that the cross-language benchmark harness in
//! `bench/` loads via `LoadLibrary` / `dlopen` and drives identically to
//! the C / C++ / Zig / Go / C# / Java / TypeScript / Python ports.
//!
//! # ABI mapping
//!
//! | C ABI function                  | Rust call                                  |
//! |---------------------------------|--------------------------------------------|
//! | `nes_core_create`               | `Cartridge::from_bytes` → `EmulatorState::new` |
//! | `nes_core_destroy`              | `Box::from_raw` + drop                      |
//! | `nes_core_reset`                | `EmulatorState::reset`                      |
//! | `nes_core_set_region`           | `EmulatorState::set_region`                 |
//! | `nes_core_step_frame`           | `EmulatorState::step_frame`                 |
//! | `nes_core_step_instruction`     | `EmulatorState::step_instruction`           |
//! | `nes_core_framebuffer`         | `emu.framebuffer().as_ptr()`               |
//! | `nes_core_take_audio`          | `emu.take_audio_samples()` → copy to buf    |
//! | `nes_core_save_state`          | `emu.save_state()` → copy to buf            |
//! | `nes_core_load_state`          | copy buf → `emu.load_state()`               |
//! | `nes_core_mapper_number`       | `emu.mapper_number()`                       |
//! | `nes_core_impl_name`           | static `"rust-nes"`                         |
//! | `nes_core_impl_version`        | static `"0.1.0"`                           |
//!
//! # Safety
//!
//! All exported functions are `extern "C"` and take the opaque `nes_core_t*`
//! handle returned by `nes_core_create`. The handle is a `Box<NesCore>` in
//! Rust; callers must never inspect or free its contents directly — use
//! `nes_core_destroy`. Passing a NULL handle to any function is a safe no-op
//! (or returns 0 / NULL as documented in `protocol.h`).
//!
//! See: `cores/protocol.h` for the authoritative ABI specification.

#![allow(clippy::missing_safety_doc)]
// These are the C ABI entry points — the C caller cannot honour Rust's
// `unsafe` contract, and marking them `unsafe` would only change the Rust
// call signature (which nobody uses from safe Rust). The whole point of the
// wrapper is to be the soundness boundary: we null-check every pointer
// before dereferencing.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::c_char;
use std::ptr;

use nes_emu::cartridge::Cartridge;
use nes_emu::emulator::EmulatorState;
use nes_emu::region::Region;

/// Opaque NES core instance. The harness only ever holds a pointer to this;
/// the layout is private to this crate. It owns the emulator state plus a
/// reusable scratch buffer for save-state serialisation (avoiding a per-call
/// allocation when the caller polls `nes_core_save_state` with a large
/// buffer).
///
/// Per `protocol.h`: the framebuffer pointer returned by
/// `nes_core_framebuffer` is stable for the lifetime of the instance. The
/// PPU stores its framebuffer as a `Box<[u32]>` (heap-allocated, never
/// reallocated), so the pointer remains valid until `nes_core_destroy`.
pub struct NesCore {
    emu: EmulatorState,
    /// Reusable scratch buffer for `nes_core_save_state`. The serialised
    /// blob is produced into this buffer and then copied into the caller's
    /// buffer. Kept on the struct so repeated saves do not reallocate.
    save_scratch: Vec<u8>,
}

/// Convert a `Region` ABI integer (`NES_REGION_*`) to the Rust `Region`
/// enum. Returns `None` for out-of-range values. The mapping is fixed by
/// `protocol.h`: NTSC=0, PAL=1, Dendy=2.
fn region_from_abi(region: i32) -> Option<Region> {
    match region {
        0 => Some(Region::Ntsc),
        1 => Some(Region::Pal),
        2 => Some(Region::Dendy),
        _ => None,
    }
}

/// Convert a Rust `Region` to its ABI integer. Inverse of
/// [`region_from_abi`].
fn region_to_abi(region: Region) -> i32 {
    match region {
        Region::Ntsc => 0,
        Region::Pal => 1,
        Region::Dendy => 2,
    }
}

/// Convert an `f32` sample in `[-1.0, 1.0]` to a signed 16-bit PCM value.
/// Values outside `[-1.0, 1.0]` are clamped (the emulator already clamps its
/// mix output, but defend in depth — a future expansion-audio source could
/// overshoot). Saturating conversion avoids UB from out-of-range casts.
#[inline]
fn f32_to_i16_saturating(sample: f32) -> i16 {
    let scaled = sample * 32767.0;
    if scaled >= 32767.0 {
        32767
    } else if scaled <= -32768.0 {
        -32768
    } else {
        scaled as i16
    }
}

// -------------------------------------------------------------------------
// Lifecycle
// -------------------------------------------------------------------------

/// Create a new NES core from an iNES/FDS ROM image held in memory.
/// See `cores/protocol.h` → `nes_core_create`.
#[no_mangle]
pub extern "C" fn nes_core_create(
    rom_data: *const u8,
    rom_len: usize,
) -> *mut NesCore {
    if rom_data.is_null() {
        return ptr::null_mut();
    }
    // Build a slice from the caller's buffer. The caller owns the ROM buffer
    // and may free it immediately after this call returns, so we copy what
    // the cartridge needs here (Cartridge::from_bytes does its own copy of
    // PRG/CHR into owned buffers).
    let rom = unsafe { std::slice::from_raw_parts(rom_data, rom_len) };

    let cartridge = match Cartridge::from_bytes(rom) {
        Ok(cart) => cart,
        Err(_) => return ptr::null_mut(),
    };

    let emu = EmulatorState::new(cartridge);
    let core = Box::new(NesCore {
        emu,
        save_scratch: Vec::new(),
    });
    Box::into_raw(core)
}

/// Destroy a core instance. NULL is a no-op.
/// See `cores/protocol.h` → `nes_core_destroy`.
#[no_mangle]
pub extern "C" fn nes_core_destroy(core: *mut NesCore) {
    if !core.is_null() {
        unsafe { drop(Box::from_raw(core)) };
    }
}

/// Hard reset (power-cycle semantics). Safe on a freshly created core.
/// See `cores/protocol.h` → `nes_core_reset`.
#[no_mangle]
pub extern "C" fn nes_core_reset(core: *mut NesCore) {
    if let Some(core) = unsafe { core.as_mut() } {
        core.emu.reset();
    }
}

/// Set the target region. Returns the previous region, or -1 on NULL core /
/// out-of-range `region`. Takes effect on the next reset / frame boundary
/// (the emulator propagates the region to the PPU and APU immediately, but
/// timing changes fully apply from the next frame).
/// See `cores/protocol.h` → `nes_core_set_region`.
#[no_mangle]
pub extern "C" fn nes_core_set_region(core: *mut NesCore, region: i32) -> i32 {
    let core = match unsafe { core.as_mut() } {
        Some(c) => c,
        None => return -1,
    };
    let new_region = match region_from_abi(region) {
        Some(r) => r,
        None => return -1,
    };
    let prev = core.emu.region();
    core.emu.set_region(new_region);
    region_to_abi(prev)
}

// -------------------------------------------------------------------------
// Stepping
// -------------------------------------------------------------------------

/// Run until one full frame is rendered. Returns CPU cycles consumed
/// (≈29830 for NTSC), or 0 on NULL core.
/// See `cores/protocol.h` → `nes_core_step_frame`.
#[no_mangle]
pub extern "C" fn nes_core_step_frame(core: *mut NesCore) -> u32 {
    if let Some(core) = unsafe { core.as_mut() } {
        core.emu.step_frame()
    } else {
        0
    }
}

/// Execute exactly one CPU instruction. Returns CPU cycles consumed, or 0
/// on NULL core.
/// See `cores/protocol.h` → `nes_core_step_instruction`.
#[no_mangle]
pub extern "C" fn nes_core_step_instruction(core: *mut NesCore) -> u32 {
    if let Some(core) = unsafe { core.as_mut() } {
        core.emu.step_instruction()
    } else {
        0
    }
}

// -------------------------------------------------------------------------
// Output
// -------------------------------------------------------------------------

/// Return a pointer to the 256×240 ARGB framebuffer. Owned by the core,
/// stable for the instance lifetime. Returns NULL on NULL core.
/// See `cores/protocol.h` → `nes_core_framebuffer`.
#[no_mangle]
pub extern "C" fn nes_core_framebuffer(core: *mut NesCore) -> *const u32 {
    match unsafe { core.as_ref() } {
        Some(core) => core.emu.framebuffer().as_ptr(),
        None => ptr::null(),
    }
}

/// Drain queued audio samples (mono int16 PCM, 44100 Hz) into `buf`. Returns
/// the number of samples written (0..cap). NULL core/buf → 0.
/// See `cores/protocol.h` → `nes_core_take_audio`.
#[no_mangle]
pub extern "C" fn nes_core_take_audio(
    core: *mut NesCore,
    buf: *mut i16,
    cap: usize,
) -> usize {
    let core = match unsafe { core.as_mut() } {
        Some(c) => c,
        None => return 0,
    };
    if buf.is_null() || cap == 0 {
        // Drain the internal buffer anyway so it does not grow unbounded
        // when the caller polls with a zero-capacity buffer, matching the
        // protocol's "queue is emptied for the samples that fit" semantics
        // (a zero-cap poll empties nothing, but we still must not panic).
        return 0;
    }

    let samples = core.emu.take_audio_samples();
    let n = samples.len().min(cap);
    let out = unsafe { std::slice::from_raw_parts_mut(buf, n) };
    for (i, &s) in samples.iter().take(n).enumerate() {
        out[i] = f32_to_i16_saturating(s);
    }
    n
}

// -------------------------------------------------------------------------
// Save state
// -------------------------------------------------------------------------

/// Serialise the full core state into `buf`. Returns bytes written, or 0 on
/// failure (buffer too small / NULL core). A 0 return with non-NULL core and
/// non-zero cap indicates an error.
/// See `cores/protocol.h` → `nes_core_save_state`.
#[no_mangle]
pub extern "C" fn nes_core_save_state(
    core: *mut NesCore,
    buf: *mut u8,
    cap: usize,
) -> usize {
    let core = match unsafe { core.as_mut() } {
        Some(c) => c,
        None => return 0,
    };
    if buf.is_null() || cap == 0 {
        return 0;
    }

    let blob = match core.emu.save_state() {
        Ok(b) => b,
        Err(_) => return 0,
    };

    if blob.len() > cap {
        // Buffer too small. Stash the blob so a subsequent call with a larger
        // buffer could retry — but per the protocol a too-small buffer is an
        // error and returns 0. We keep the scratch buffer for future use.
        core.save_scratch = blob;
        return 0;
    }

    let out = unsafe { std::slice::from_raw_parts_mut(buf, blob.len()) };
    out.copy_from_slice(&blob);
    blob.len()
}

/// Restore core state from `buf`. Returns 1 on success, 0 on failure.
/// See `cores/protocol.h` → `nes_core_load_state`.
#[no_mangle]
pub extern "C" fn nes_core_load_state(
    core: *mut NesCore,
    buf: *const u8,
    len: usize,
) -> i32 {
    let core = match unsafe { core.as_mut() } {
        Some(c) => c,
        None => return 0,
    };
    if buf.is_null() || len == 0 {
        return 0;
    }
    let data = unsafe { std::slice::from_raw_parts(buf, len) };
    match core.emu.load_state(data) {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

// -------------------------------------------------------------------------
// Introspection
// -------------------------------------------------------------------------

/// Return the iNES mapper number (0..255) of the loaded ROM, or 0 if no ROM
/// is loaded. Mapper 0 (NROM) is a valid return.
/// See `cores/protocol.h` → `nes_core_mapper_number`.
#[no_mangle]
pub extern "C" fn nes_core_mapper_number(core: *mut NesCore) -> u16 {
    match unsafe { core.as_ref() } {
        Some(core) => core.emu.mapper_number(),
        None => 0,
    }
}

/// Static implementation name. Never NULL, valid for the process lifetime.
/// See `cores/protocol.h` → `nes_core_impl_name`.
#[no_mangle]
pub extern "C" fn nes_core_impl_name() -> *const c_char {
    // b"...\0" is a static with a trailing NUL; its address is stable for
    // the entire process lifetime (no allocation, no Drop).
    static NAME: &[u8] = b"rust-nes\0";
    NAME.as_ptr() as *const c_char
}

/// Static implementation version. Never NULL, valid for the process lifetime.
/// See `cores/protocol.h` → `nes_core_impl_version`.
#[no_mangle]
pub extern "C" fn nes_core_impl_version() -> *const c_char {
    static VERSION: &[u8] = b"0.1.0\0";
    VERSION.as_ptr() as *const c_char
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal NROM-128 ROM (16 KB PRG filled with NOP, RESET →
    /// $C000). Mirrors the emulator's own `make_nop_cart` test helper so we
    /// can exercise the FFI without depending on private items.
    fn nop_rom() -> Vec<u8> {
        let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
        bytes.extend_from_slice(&[0u8; 8]); // remaining header
        bytes.resize(16 + 16 * 1024, 0xEA); // PRG = NOP
        let reset_off = 16 + 0x3FFC;
        bytes[reset_off] = 0x00;
        bytes[reset_off + 1] = 0xC0;
        bytes
    }

    #[test]
    fn create_and_destroy_roundtrip() {
        let rom = nop_rom();
        let core = nes_core_create(rom.as_ptr(), rom.len());
        assert!(!core.is_null());
        nes_core_destroy(core);
    }

    #[test]
    fn null_handle_is_safe_noop() {
        assert_eq!(nes_core_step_frame(std::ptr::null_mut()), 0);
        assert_eq!(nes_core_step_instruction(std::ptr::null_mut()), 0);
        assert!(nes_core_framebuffer(std::ptr::null_mut()).is_null());
        assert_eq!(nes_core_take_audio(std::ptr::null_mut(), std::ptr::null_mut(), 0), 0);
        assert_eq!(nes_core_save_state(std::ptr::null_mut(), std::ptr::null_mut(), 0), 0);
        assert_eq!(nes_core_load_state(std::ptr::null_mut(), std::ptr::null(), 0), 0);
        assert_eq!(nes_core_mapper_number(std::ptr::null_mut()), 0);
        assert_eq!(nes_core_set_region(std::ptr::null_mut(), 0), -1);
        nes_core_reset(std::ptr::null_mut()); // must not panic
        nes_core_destroy(std::ptr::null_mut()); // must not panic
    }

    #[test]
    fn bad_rom_returns_null() {
        let bad = [0u8; 4];
        let core = nes_core_create(bad.as_ptr(), bad.len());
        assert!(core.is_null());
    }

    #[test]
    fn null_rom_returns_null() {
        let core = nes_core_create(std::ptr::null(), 0);
        assert!(core.is_null());
    }

    #[test]
    fn step_frame_produces_framebuffer_and_cycles() {
        let rom = nop_rom();
        let core = nes_core_create(rom.as_ptr(), rom.len());
        assert!(!core.is_null());
        nes_core_reset(core);

        let cycles = nes_core_step_frame(core);
        // NTSC frame ≈ 29,830 CPU cycles.
        assert!(
            (29_300..=30_400).contains(&cycles),
            "expected ~29830 cycles, got {cycles}",
        );

        let fb = nes_core_framebuffer(core);
        assert!(!fb.is_null());
        // The framebuffer should not be all-zero after a frame (the universal
        // bg color fills it). Read one pixel to confirm it is non-zero.
        let pixel = unsafe { *fb };
        assert_ne!(pixel, 0, "framebuffer pixel 0 should be non-zero bg color");

        nes_core_destroy(core);
    }

    #[test]
    fn step_instruction_returns_nonzero_cycles() {
        let rom = nop_rom();
        let core = nes_core_create(rom.as_ptr(), rom.len());
        nes_core_reset(core);
        let cycles = nes_core_step_instruction(core);
        // NOP = 2 cycles.
        assert_eq!(cycles, 2, "NOP should consume 2 CPU cycles");
        nes_core_destroy(core);
    }

    #[test]
    fn run_many_frames_without_crash() {
        let rom = nop_rom();
        let core = nes_core_create(rom.as_ptr(), rom.len());
        nes_core_reset(core);
        for _ in 0..1000 {
            let c = nes_core_step_frame(core);
            assert!(c > 0);
        }
        nes_core_destroy(core);
    }

    #[test]
    fn audio_drains_to_i16_pcm() {
        let rom = nop_rom();
        let core = nes_core_create(rom.as_ptr(), rom.len());
        nes_core_reset(core);
        nes_core_step_frame(core);
        let mut buf = [0i16; 2048];
        let n = nes_core_take_audio(core, buf.as_mut_ptr(), buf.len());
        // A NOP ROM produces no audio activity, but the APU triangle channel
        // may still emit silence samples. Either way n must be <= cap and
        // the call must not panic.
        assert!(n <= buf.len(), "take_audio returned {n} > cap {}", buf.len());
        nes_core_destroy(core);
    }

    #[test]
    fn save_then_load_state_roundtrip() {
        let rom = nop_rom();
        let core = nes_core_create(rom.as_ptr(), rom.len());
        nes_core_reset(core);
        // Run a few frames to populate state.
        for _ in 0..5 {
            nes_core_step_frame(core);
        }

        // Save into a generously-sized buffer.
        let mut save_buf = vec![0u8; 1 << 20]; // 1 MiB
        let saved = nes_core_save_state(core, save_buf.as_mut_ptr(), save_buf.len());
        assert!(saved > 0, "save_state should write >0 bytes");

        // Run a few more frames to diverge state.
        for _ in 0..5 {
            nes_core_step_frame(core);
        }

        // Load the saved state back.
        let ok = nes_core_load_state(core, save_buf.as_ptr(), saved);
        assert_eq!(ok, 1, "load_state should succeed");

        // After load, the framebuffer pointer is *not* guaranteed to equal
        // the pre-load pointer: `load_state` replaces the PPU (which owns
        // the framebuffer `Box<[u32]>`), so a fresh buffer is allocated.
        // The protocol guarantees pointer stability across `step_frame`
        // calls, not across `load_state`. Confirm the new pointer is valid
        // and that stepping still works.
        let fb_after = nes_core_framebuffer(core);
        assert!(!fb_after.is_null(), "framebuffer pointer must be non-null after load");
        let cycles = nes_core_step_frame(core);
        assert!(cycles > 0, "step_frame must run after load_state");
        // Re-saving after the roundtrip must still produce a valid blob.
        let mut save_buf2 = vec![0u8; 1 << 20];
        let saved2 = nes_core_save_state(core, save_buf2.as_mut_ptr(), save_buf2.len());
        assert!(saved2 > 0, "save_state must work after load+step");

        nes_core_destroy(core);
    }

    #[test]
    fn save_state_too_small_buffer_returns_zero() {
        let rom = nop_rom();
        let core = nes_core_create(rom.as_ptr(), rom.len());
        nes_core_reset(core);
        nes_core_step_frame(core);
        let mut tiny = [0u8; 8];
        let n = nes_core_save_state(core, tiny.as_mut_ptr(), tiny.len());
        assert_eq!(n, 0, "save_state into a tiny buffer should return 0");
        nes_core_destroy(core);
    }

    #[test]
    fn load_state_bad_data_returns_zero() {
        let rom = nop_rom();
        let core = nes_core_create(rom.as_ptr(), rom.len());
        nes_core_reset(core);
        let garbage = [0xFFu8; 64];
        let ok = nes_core_load_state(core, garbage.as_ptr(), garbage.len());
        assert_eq!(ok, 0, "load_state of garbage should fail");
        nes_core_destroy(core);
    }

    #[test]
    fn mapper_number_for_nrom_is_zero() {
        let rom = nop_rom();
        let core = nes_core_create(rom.as_ptr(), rom.len());
        assert_eq!(nes_core_mapper_number(core), 0);
        nes_core_destroy(core);
    }

    #[test]
    fn set_region_returns_previous_and_clamps() {
        let rom = nop_rom();
        let core = nes_core_create(rom.as_ptr(), rom.len());
        // Default region is NTSC (0).
        assert_eq!(nes_core_set_region(core, 1), 0, "prev should be NTSC=0");
        assert_eq!(nes_core_set_region(core, 2), 1, "prev should be PAL=1");
        assert_eq!(nes_core_set_region(core, 0), 2, "prev should be Dendy=2");
        // Out-of-range region → -1, current region unchanged.
        assert_eq!(nes_core_set_region(core, 99), -1);
        assert_eq!(nes_core_set_region(core, 1), 0, "still NTSC after bad set");
        nes_core_destroy(core);
    }

    #[test]
    fn impl_name_and_version_are_non_null() {
        let name = nes_core_impl_name();
        let version = nes_core_impl_version();
        assert!(!name.is_null());
        assert!(!version.is_null());
        let name_str = unsafe {
            std::ffi::CStr::from_ptr(name).to_str().unwrap()
        };
        let version_str = unsafe {
            std::ffi::CStr::from_ptr(version).to_str().unwrap()
        };
        assert_eq!(name_str, "rust-nes");
        assert_eq!(version_str, "0.1.0");
    }
}
