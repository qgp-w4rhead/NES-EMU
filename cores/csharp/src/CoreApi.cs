using System;
using System.Collections.Concurrent;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Threading;

namespace NesCore;

// NES polyglot core C ABI shim (cores/protocol.h).
// Implements the 13-function C ABI via [UnmanagedCallersOnly] so the
// benchmark harness in bench/ can load nes_core_csharp.dll and drive it
// identically to the C / C++ / Zig / Go cores.
//
// The opaque nes_core_t handle is a pointer-sized integer (nint) key into
// a static handle-map registry (ConcurrentDictionary<nint, Emulator>) so
// the GC can see and root the managed Emulator objects. The framebuffer is
// a native (Marshal.AllocHGlobal) buffer copied from the managed PPU
// framebuffer after each step_frame, so the returned pointer is stable and
// owned by the core. No exceptions cross the FFI boundary (try/catch
// returns 0/null/false/-1 on exception).
public static class CoreApi
{
    private const string ImplNameStr = "csharp-nes";
    private const string ImplVersionStr = "0.1.0";

    // ConcurrentDictionary: protocol.h requires distinct instances to be
    // usable from distinct threads concurrently. Reads (Lookup) happen
    // unlocked; writes (Register/Unregister) are atomic on the CDT.
    private static readonly ConcurrentDictionary<nint, Emulator> s_handles = new();
    private static int s_nextHandle = 1;

    // Native framebuffer buffers, one per emulator (keyed by handle).
    private static readonly ConcurrentDictionary<nint, IntPtr> s_framebuffers = new();
    private const int FramebufferBytes = 256 * 240 * 4;

    // Native impl_name/version strings (allocated once, atomic publish).
    private static IntPtr s_implNamePtr = IntPtr.Zero;
    private static IntPtr s_implVersionPtr = IntPtr.Zero;

    private static nint RegisterEmulator(Emulator emu)
    {
        int h = Interlocked.Increment(ref s_nextHandle) - 1;
        s_handles[(nint)h] = emu;
        IntPtr fb = Marshal.AllocHGlobal(FramebufferBytes);
        unsafe
        {
            Unsafe.InitBlockUnaligned((void*)fb, 0, (uint)FramebufferBytes);
        }
        s_framebuffers[(nint)h] = fb;
        return (nint)h;
    }

    private static Emulator Lookup(nint h)
        => s_handles.TryGetValue(h, out var e) ? e : null;

    private static void Unregister(nint h)
    {
        s_handles.TryRemove(h, out var emu);
        if (s_framebuffers.TryRemove(h, out var fb))
            Marshal.FreeHGlobal(fb);
        emu?.Destroy();
    }

    private static unsafe void SyncFramebuffer(nint h, Emulator emu)
    {
        if (!s_framebuffers.TryGetValue(h, out IntPtr fb) || fb == IntPtr.Zero) return;
        uint[] src = emu.Bus.Ppu.Framebuffer;
        fixed (uint* pSrc = src)
        {
            Buffer.MemoryCopy(pSrc, (void*)fb, FramebufferBytes, FramebufferBytes);
        }
    }

    private static IntPtr GetImplNamePtr()
    {
        IntPtr p = Interlocked.CompareExchange(ref s_implNamePtr, IntPtr.Zero, IntPtr.Zero);
        if (p == IntPtr.Zero)
        {
            p = Marshal.StringToHGlobalAnsi(ImplNameStr);
            IntPtr prev = Interlocked.CompareExchange(ref s_implNamePtr, p, IntPtr.Zero);
            if (prev != IntPtr.Zero) { Marshal.FreeHGlobal(p); p = prev; }
        }
        return p;
    }

    private static IntPtr GetImplVersionPtr()
    {
        IntPtr p = Interlocked.CompareExchange(ref s_implVersionPtr, IntPtr.Zero, IntPtr.Zero);
        if (p == IntPtr.Zero)
        {
            p = Marshal.StringToHGlobalAnsi(ImplVersionStr);
            IntPtr prev = Interlocked.CompareExchange(ref s_implVersionPtr, p, IntPtr.Zero);
            if (prev != IntPtr.Zero) { Marshal.FreeHGlobal(p); p = prev; }
        }
        return p;
    }

    // ---- Lifecycle ----

    [UnmanagedCallersOnly(EntryPoint = "nes_core_create")]
    public unsafe static void* Create(byte* romData, ulong romLen)
    {
        try
        {
            if (romData == null || romLen == 0ul) return null;
            int len = (int)romLen;
            byte[] bytes = new byte[len];
            for (int i = 0; i < len; ++i) bytes[i] = romData[i];
            int rc = Cartridge.FromBytes(bytes, len, out Cartridge cart);
            if (rc != 0) return null;
            var emu = new Emulator();
            emu.Init();
            emu.Cartridge = cart;
            emu.Bus.InitWithCartridge(cart);
            emu.Cpu.Bus = emu.Bus;
            emu.Cpu.Reset();
            nint h = RegisterEmulator(emu);
            return (void*)h;
        }
        catch { return null; }
    }

    [UnmanagedCallersOnly(EntryPoint = "nes_core_destroy")]
    public unsafe static void Destroy(void* core)
    {
        try
        {
            if (core == null) return;
            Unregister((nint)core);
        }
        catch { }
    }

    [UnmanagedCallersOnly(EntryPoint = "nes_core_reset")]
    public unsafe static void Reset(void* core)
    {
        try
        {
            if (core == null) return;
            var emu = Lookup((nint)core);
            if (emu != null) emu.Reset();
        }
        catch { }
    }

    [UnmanagedCallersOnly(EntryPoint = "nes_core_set_region")]
    public unsafe static int SetRegion(void* core, int region)
    {
        try
        {
            if (core == null) return -1;
            if (region < 0 || region > (int)Region.Dendy) return -1;
            var emu = Lookup((nint)core);
            if (emu == null) return -1;
            int prev = (int)emu.Region;
            emu.SetRegion((Region)region);
            return prev;
        }
        catch { return -1; }
    }

    // ---- Stepping ----

    [UnmanagedCallersOnly(EntryPoint = "nes_core_step_frame")]
    public unsafe static uint StepFrame(void* core)
    {
        try
        {
            if (core == null) return 0u;
            nint h = (nint)core;
            var emu = Lookup(h);
            if (emu == null) return 0u;
            uint cycles = emu.StepFrame();
            SyncFramebuffer(h, emu);
            return cycles;
        }
        catch { return 0u; }
    }

    [UnmanagedCallersOnly(EntryPoint = "nes_core_step_instruction")]
    public unsafe static uint StepInstruction(void* core)
    {
        try
        {
            if (core == null) return 0u;
            var emu = Lookup((nint)core);
            if (emu == null) return 0u;
            // Do NOT sync framebuffer here — step_instruction is for
            // instruction-level benchmarking and the framebuffer sync
            // (256x240 bulk copy) would dominate the measurement.
            return emu.StepInstruction();
        }
        catch { return 0u; }
    }

    // ---- Output ----

    [UnmanagedCallersOnly(EntryPoint = "nes_core_framebuffer")]
    public unsafe static uint* Framebuffer(void* core)
    {
        try
        {
            if (core == null) return null;
            nint h = (nint)core;
            lock (s_framebuffers)
            {
                if (s_framebuffers.TryGetValue(h, out var fb))
                    return (uint*)fb;
            }
            return null;
        }
        catch { return null; }
    }

    [UnmanagedCallersOnly(EntryPoint = "nes_core_take_audio")]
    public unsafe static ulong TakeAudio(void* core, short* buf, ulong cap)
    {
        try
        {
            if (core == null || buf == null || cap == 0ul) return 0ul;
            var emu = Lookup((nint)core);
            if (emu == null) return 0ul;
            int capInt = (int)cap;
            // Read directly from the emulator's audio buffer (no temp alloc).
            uint n = emu.AudioBufferCount;
            if (n > cap) n = (uint)capInt;
            for (uint i = 0; i < n; ++i)
            {
                float s = emu.AudioBuffer[i];
                if (s > 1.0f) s = 1.0f;
                if (s < -1.0f) s = -1.0f;
                int v;
                if (s >= 0.0f)
                {
                    v = (int)(s * 32767.0f);
                    if (v > 32767) v = 32767;
                }
                else
                {
                    v = (int)(s * 32768.0f);
                    if (v < -32768) v = -32768;
                }
                buf[i] = (short)v;
            }
            emu.AudioBufferCount = 0;
            return (ulong)n;
        }
        catch { return 0ul; }
    }

    // ---- Save state ----

    [UnmanagedCallersOnly(EntryPoint = "nes_core_save_state")]
    public unsafe static ulong SaveStateExport(void* core, byte* buf, ulong cap)
    {
        try
        {
            if (core == null || buf == null || cap == 0ul) return 0ul;
            var emu = Lookup((nint)core);
            if (emu == null) return 0ul;
            int capInt = (int)cap;
            byte[] managed = new byte[capInt];
            uint written = SaveState.Save(emu, managed, (uint)capInt);
            if (written == 0u) return 0ul;
            for (uint i = 0; i < written; ++i) buf[i] = managed[i];
            return (ulong)written;
        }
        catch { return 0ul; }
    }

    [UnmanagedCallersOnly(EntryPoint = "nes_core_load_state")]
    public unsafe static int LoadState(void* core, byte* buf, ulong len)
    {
        try
        {
            if (core == null || buf == null || len == 0ul) return 0;
            var emu = Lookup((nint)core);
            if (emu == null) return 0;
            int lenInt = (int)len;
            byte[] managed = new byte[lenInt];
            for (int i = 0; i < lenInt; ++i) managed[i] = buf[i];
            return SaveState.Load(emu, managed, (uint)lenInt) ? 1 : 0;
        }
        catch { return 0; }
    }

    // ---- Introspection ----

    [UnmanagedCallersOnly(EntryPoint = "nes_core_mapper_number")]
    public unsafe static ushort MapperNumber(void* core)
    {
        try
        {
            if (core == null) return 0;
            var emu = Lookup((nint)core);
            if (emu == null) return 0;
            return emu.MapperNumber();
        }
        catch { return 0; }
    }

    [UnmanagedCallersOnly(EntryPoint = "nes_core_impl_name")]
    public unsafe static byte* ImplNameExport()
    {
        try { return (byte*)GetImplNamePtr(); }
        catch { return null; }
    }

    [UnmanagedCallersOnly(EntryPoint = "nes_core_impl_version")]
    public unsafe static byte* ImplVersionExport()
    {
        try { return (byte*)GetImplVersionPtr(); }
        catch { return null; }
    }
}
