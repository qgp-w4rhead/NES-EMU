package nes.core;

import java.util.HashMap;
import java.util.concurrent.atomic.AtomicInteger;

import org.graalvm.nativeimage.UnmanagedMemory;
import org.graalvm.nativeimage.c.function.CEntryPoint;
import org.graalvm.nativeimage.c.type.CCharPointer;
import org.graalvm.nativeimage.c.type.CIntPointer;
import org.graalvm.nativeimage.c.type.CShortPointer;
import org.graalvm.nativeimage.c.type.CTypeConversion;
import org.graalvm.nativeimage.c.type.VoidPointer;
import org.graalvm.nativeimage.IsolateThread;
import org.graalvm.word.Pointer;
import org.graalvm.word.UnsignedWord;
import org.graalvm.word.WordFactory;

// NES polyglot core C ABI shim (cores/protocol.h).
// Implements the 13-function C ABI via GraalVM @CEntryPoint so the
// benchmark harness in bench/ can load nes_core_java.dll and drive it
// identically to the C / C++ / Zig / Go / C# cores.
//
// The opaque nes_core_t handle is a pointer-sized integer key into a static
// handle-map registry (HashMap<Integer, Emulator>) so the GC can see and
// root the managed Emulator objects. The framebuffer is a native buffer
// (allocated via CTypeConversion.toCBytes, kept alive for the instance
// lifetime) copied from the managed PPU framebuffer after each step_frame.
// No exceptions cross the FFI boundary (try/catch returns 0/null/-1).
public final class CoreApi {
    private static final String IMPL_NAME_STR = "java-nes";
    private static final String IMPL_VERSION_STR = "0.1.0";
    private static final int FRAMEBUFFER_SIZE = 256 * 240;
    private static final int FRAMEBUFFER_BYTES = FRAMEBUFFER_SIZE * 4;

    // Handle registry: maps integer handle -> Emulator instance.
    private static final HashMap<Integer, Emulator> s_handles = new HashMap<>();
    private static final AtomicInteger s_nextHandle = new AtomicInteger(1);

    // Native framebuffer buffers, one per emulator (keyed by handle).
    // Store raw long addresses (Word types cannot be stored in HashMap under
    // native-image — "Expected Object but got Word"). Reconstruct CIntPointer
    // via WordFactory.pointer(addr) at call time.
    private static final HashMap<Integer, Long> s_fbAddrs = new HashMap<>();

    // Native impl_name/version string holders (allocated once, kept forever).
    private static CTypeConversion.CCharPointerHolder s_implNameHolder;
    private static CTypeConversion.CCharPointerHolder s_implVersionHolder;

    private CoreApi() {}

    public static void main(String[] args) {}

    private static synchronized int registerEmulator(Emulator emu) {
        int h = s_nextHandle.getAndIncrement();
        s_handles.put(h, emu);
        Pointer fbPtr = UnmanagedMemory.malloc(WordFactory.unsigned(FRAMEBUFFER_BYTES));
        s_fbAddrs.put(h, fbPtr.rawValue());
        return h;
    }

    private static Emulator lookup(int h) {
        synchronized (s_handles) { return s_handles.get(h); }
    }

    private static synchronized void unregister(int h) {
        Emulator emu = s_handles.remove(h);
        Long addr = s_fbAddrs.remove(h);
        if (addr != null) {
            Pointer ptr = WordFactory.pointer(addr);
            UnmanagedMemory.free(ptr);
        }
        if (emu != null) emu.destroy();
    }

    private static void syncFramebuffer(int h, Emulator emu) {
        Long addr = s_fbAddrs.get(h);
        if (addr == null) return;
        CIntPointer ptr = WordFactory.pointer(addr);
        int[] src = emu.bus.ppu.framebuffer;
        for (int i = 0; i < FRAMEBUFFER_SIZE; ++i)
            ptr.write(i, src[i]);
    }

    private static CCharPointer getImplNamePtr() {
        if (s_implNameHolder == null) {
            synchronized (CoreApi.class) {
                if (s_implNameHolder == null) {
                    s_implNameHolder = CTypeConversion.toCString(IMPL_NAME_STR);
                }
            }
        }
        return s_implNameHolder.get();
    }

    private static CCharPointer getImplVersionPtr() {
        if (s_implVersionHolder == null) {
            synchronized (CoreApi.class) {
                if (s_implVersionHolder == null) {
                    s_implVersionHolder = CTypeConversion.toCString(IMPL_VERSION_STR);
                }
            }
        }
        return s_implVersionHolder.get();
    }

    // ---- Lifecycle ----

    @CEntryPoint(name = "java_create")
    public static VoidPointer create(IsolateThread thread, CCharPointer romData, UnsignedWord romLen) {
        try {
            if (romData.isNull() || romLen.rawValue() == 0) return WordFactory.nullPointer();
            int len = (int) romLen.rawValue();
            byte[] bytes = new byte[len];
            for (int i = 0; i < len; ++i) bytes[i] = romData.read(i);
            Cartridge[] outCart = new Cartridge[1];
            int rc = Cartridge.fromBytes(bytes, len, outCart);
            if (rc != 0) return WordFactory.nullPointer();
            Emulator emu = new Emulator();
            emu.init();
            emu.cartridge = outCart[0];
            emu.bus.initWithCartridge(outCart[0]);
            emu.cpu.bus = emu.bus;
            emu.cpu.reset();
            int h = registerEmulator(emu);
            return WordFactory.pointer(h);
        } catch (Throwable t) {
            return WordFactory.nullPointer();
        }
    }

    @CEntryPoint(name = "java_destroy")
    public static void destroy(IsolateThread thread, VoidPointer core) {
        try {
            if (core.isNull()) return;
            unregister((int) core.rawValue());
        } catch (Throwable t) {}
    }

    @CEntryPoint(name = "java_reset")
    public static void reset(IsolateThread thread, VoidPointer core) {
        try {
            if (core.isNull()) return;
            Emulator emu = lookup((int) core.rawValue());
            if (emu != null) emu.reset();
        } catch (Throwable t) {}
    }

    @CEntryPoint(name = "java_set_region")
    public static int setRegion(IsolateThread thread, VoidPointer core, int region) {
        try {
            if (core.isNull()) return -1;
            if (region < 0 || region > Region.DENDY) return -1;
            Emulator emu = lookup((int) core.rawValue());
            if (emu == null) return -1;
            int prev = emu.region;
            emu.setRegion(region);
            return prev;
        } catch (Throwable t) {
            return -1;
        }
    }

    // ---- Stepping ----

    @CEntryPoint(name = "java_step_frame")
    public static int stepFrame(IsolateThread thread, VoidPointer core) {
        try {
            if (core.isNull()) return 0;
            int h = (int) core.rawValue();
            Emulator emu = lookup(h);
            if (emu == null) return 0;
            int cycles = emu.stepFrame();
            syncFramebuffer(h, emu);
            return cycles;
        } catch (Throwable t) {
            return 0;
        }
    }

    @CEntryPoint(name = "java_step_instruction")
    public static int stepInstruction(IsolateThread thread, VoidPointer core) {
        try {
            if (core.isNull()) return 0;
            Emulator emu = lookup((int) core.rawValue());
            if (emu == null) return 0;
            return emu.stepInstruction();
        } catch (Throwable t) {
            return 0;
        }
    }

    // ---- Output ----

    @CEntryPoint(name = "java_framebuffer")
    public static CIntPointer framebuffer(IsolateThread thread, VoidPointer core) {
        try {
            if (core.isNull()) return WordFactory.nullPointer();
            int h = (int) core.rawValue();
            Long addr = s_fbAddrs.get(h);
            if (addr == null) return WordFactory.nullPointer();
            return WordFactory.pointer(addr);
        } catch (Throwable t) {
            return WordFactory.nullPointer();
        }
    }

    @CEntryPoint(name = "java_take_audio")
    public static UnsignedWord takeAudio(IsolateThread thread, VoidPointer core, CShortPointer buf, UnsignedWord cap) {
        try {
            if (core.isNull() || buf.isNull() || cap.rawValue() == 0)
                return WordFactory.unsigned(0);
            Emulator emu = lookup((int) core.rawValue());
            if (emu == null) return WordFactory.unsigned(0);
            int capInt = (int) cap.rawValue();
            int n = emu.audioBufferCount;
            if (n > capInt) n = capInt;
            for (int i = 0; i < n; ++i) {
                float s = emu.audioBuffer[i];
                if (s > 1.0f) s = 1.0f;
                if (s < -1.0f) s = -1.0f;
                int v;
                if (s >= 0.0f) {
                    v = (int) (s * 32767.0f);
                    if (v > 32767) v = 32767;
                } else {
                    v = (int) (s * 32768.0f);
                    if (v < -32768) v = -32768;
                }
                buf.write(i, (short) v);
            }
            emu.audioBufferCount = 0;
            return WordFactory.unsigned(n);
        } catch (Throwable t) {
            return WordFactory.unsigned(0);
        }
    }

    // ---- Save state ----

    @CEntryPoint(name = "java_save_state")
    public static UnsignedWord saveStateExport(IsolateThread thread, VoidPointer core, CCharPointer buf, UnsignedWord cap) {
        try {
            if (core.isNull() || buf.isNull() || cap.rawValue() == 0)
                return WordFactory.unsigned(0);
            Emulator emu = lookup((int) core.rawValue());
            if (emu == null) return WordFactory.unsigned(0);
            int capInt = (int) cap.rawValue();
            byte[] managed = new byte[capInt];
            int written = SaveState.save(emu, managed, capInt);
            if (written == 0) return WordFactory.unsigned(0);
            for (int i = 0; i < written; ++i) buf.write(i, managed[i]);
            return WordFactory.unsigned(written);
        } catch (Throwable t) {
            return WordFactory.unsigned(0);
        }
    }

    @CEntryPoint(name = "java_load_state")
    public static int loadState(IsolateThread thread, VoidPointer core, CCharPointer buf, UnsignedWord len) {
        try {
            if (core.isNull() || buf.isNull() || len.rawValue() == 0) return 0;
            Emulator emu = lookup((int) core.rawValue());
            if (emu == null) return 0;
            int lenInt = (int) len.rawValue();
            byte[] managed = new byte[lenInt];
            for (int i = 0; i < lenInt; ++i) managed[i] = buf.read(i);
            return SaveState.load(emu, managed, lenInt) ? 1 : 0;
        } catch (Throwable t) {
            return 0;
        }
    }

    // ---- Introspection ----

    @CEntryPoint(name = "java_mapper_number")
    public static short mapperNumber(IsolateThread thread, VoidPointer core) {
        try {
            if (core.isNull()) return 0;
            Emulator emu = lookup((int) core.rawValue());
            if (emu == null) return 0;
            return (short) emu.mapperNumber();
        } catch (Throwable t) {
            return 0;
        }
    }

    @CEntryPoint(name = "java_impl_name")
    public static CCharPointer implNameExport(IsolateThread thread) {
        try {
            return getImplNamePtr();
        } catch (Throwable t) {
            return WordFactory.nullPointer();
        }
    }

    @CEntryPoint(name = "java_impl_version")
    public static CCharPointer implVersionExport(IsolateThread thread) {
        try {
            return getImplVersionPtr();
        } catch (Throwable t) {
            return WordFactory.nullPointer();
        }
    }
}


