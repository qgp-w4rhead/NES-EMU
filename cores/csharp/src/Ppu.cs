using System.Runtime.CompilerServices;

namespace NesCore;

// CHR pattern-table read delegate (render.rs ChrReader trait).
// The bus implements this to route reads through the cartridge.
public delegate byte ChrReader(ushort addr);

// Transient per-pixel rendering pipeline state (render.rs RenderPipeline).
public sealed class RenderPipeline
{
    public const int MaxSpritesPerScanline = 8;

    public struct ScanlineSprite
    {
        public byte OamI;
        public byte Y;
        public byte Tile;
        public byte Attr;
        public byte Sx;
    }

    public struct BgFetch
    {
        public byte Pattern;
        public byte PalSelect;
    }

    public ushort CoarseY;
    public ushort FineY;
    public ushort NtV;

    public ushort CoarseXStart;
    public ushort NtHStart;
    public byte FineXStart;
    public ushort ResyncPx;

    public bool ScanlineInitialized;
    public bool VDirty;
    public bool RenderedThisFrame;

    public ScanlineSprite[] Sprites = new ScanlineSprite[MaxSpritesPerScanline];
    public byte SpriteCount;
    public bool Overflow;
    public bool SpriteZeroHit;

    public bool SlantCorruption;
    public bool UseInaccuratePalette;
    public bool NmiRetrigger;

    public BgFetch[] FetchBuffer = new BgFetch[2];
    public byte FetchIdx;
    public bool PipelinePrimed;
}

// Picture Processing Unit (mod.rs struct Ppu).
public sealed class Ppu
{
    public const uint VramSize4k = 0x1000;
    public const uint VramSize2k = 0x0800;
    public const uint OamSize = 256;
    public const uint PaletteSize = 32;
    public const ushort ScanlinesPerFrameNtsc = 262;
    public const ushort CyclesPerScanline = 341;
    public const ushort ScanlineVblankStart = 241;
    public const ushort ScanlinePrerenderNtsc = 261;

    public const byte CtrlNmi = 0x80;
    public const byte CtrlIncrement32 = 0x04;
    public const byte CtrlSpritePattern1000 = 0x08;
    public const byte CtrlBgPattern1000 = 0x10;
    public const byte CtrlSpriteSize16 = 0x20;
    public const byte CtrlBaseNtMask = 0x03;

    public const byte MaskShowBgLeft = 0x02;
    public const byte MaskShowSpritesLeft = 0x04;
    public const byte MaskShowBg = 0x08;
    public const byte MaskShowSprites = 0x10;

    public const byte StatusVblank = 0x80;
    public const byte StatusSpriteZero = 0x40;
    public const byte StatusOverflow = 0x20;
    public const byte StatusFlagMask = 0xE0;
    public const byte StatusOpenBusMask = 0x1F;

    public const ushort NtSelectMask = 0x0C00;
    public const ushort CoarseXMask = 0x001F;
    public const ushort CoarseYMask = 0x03E0;
    public const ushort FineYMask = 0x7000;
    public const ushort NtHBit = 0x0400;
    public const ushort NtVBit = 0x0800;

    public const uint ScreenWidth = 256;
    public const uint ScreenHeight = 240;
    public const uint FramebufferSize = ScreenWidth * ScreenHeight;

    // Internal timing constants
    private const ushort VblankNmiCycle = 1;
    private const ushort VertScrollIncCycle = 256;
    private const ushort HCopyCycle = 257;
    private const ushort VCopyCycleStart = 280;
    private const ushort VCopyCycleEnd = 304;
    private const ushort HScrollIncStep = 8;
    private const ushort HScrollIncLast = 248;

    public byte PpuCtrl;
    public byte PpuMask;
    public byte OamAddr;
    public byte PpuStatus;
    public ushort V;
    public ushort T;
    public byte FineX;
    public bool W;
    public byte PpuDataBuffer;
    public byte OpenBus;

    public byte[] Vram = new byte[VramSize4k];
    public uint VramSize = VramSize2k;
    public byte[] Oam = new byte[OamSize];
    public byte[] Palette = new byte[PaletteSize];
    public uint[] Framebuffer = new uint[FramebufferSize];
    public byte[] BgPattern = new byte[ScreenWidth];

    public ushort Scanline;
    public ushort Cycle;
    public bool NmiRequest;

    public Mirroring Mirroring = Mirroring.Horizontal;
    public Region Region = Region.Ntsc;
    public RenderPipeline Render = new RenderPipeline();

    public Ppu() { }

    public void SetMirroring(Mirroring m)
    {
        uint needed = (m == Mirroring.FourScreen) ? VramSize4k : VramSize2k;
        if (VramSize != needed)
        {
            if (needed > VramSize)
                System.Array.Clear(Vram, (int)VramSize, (int)(needed - VramSize));
            VramSize = needed;
        }
        Mirroring = m;
    }

    public void SetRegion(Region r)
    {
        Region = r;
        ushort maxSl = RegionTiming.ScanlinesPerFrame(r);
        if (Scanline >= maxSl) Scanline = (ushort)(maxSl - 1);
    }

    public void SetSlantCorruption(bool enabled) => Render.SlantCorruption = enabled;
    public void SetInaccuratePalette(bool enabled) => Render.UseInaccuratePalette = enabled;
    public void SetNmiRetrigger(bool enabled) => Render.NmiRetrigger = enabled;

    public byte ReadRegister(ushort reg)
    {
        switch (reg & 0x07)
        {
            case 0: case 1: case 3: case 5: case 6: return OpenBus;
            case 2: return ReadStatus();
            case 4: return ReadOamData();
            case 7: return PpuDataBuffer;
            default: return OpenBus;
        }
    }

    public void WriteRegister(ushort reg, byte value)
    {
        OpenBus = value;
        switch (reg & 0x07)
        {
            case 0: WritePpuCtrl(value); break;
            case 1: PpuMask = value; break;
            case 2: break;
            case 3: OamAddr = value; break;
            case 4: WriteOamData(value); break;
            case 5: WritePpuScroll(value); break;
            case 6: WritePpuAddr(value); break;
            case 7: break;
        }
    }

    private byte ReadStatus()
    {
        byte result = (byte)((PpuStatus & StatusFlagMask) | (OpenBus & StatusOpenBusMask));
        PpuStatus &= (byte)(StatusVblank ^ 0xFF);
        W = false;
        OpenBus = result;
        return result;
    }

    private byte ReadOamData()
    {
        byte value = Oam[OamAddr];
        OamAddr++;
        OpenBus = value;
        return value;
    }

    private void WriteOamData(byte value)
    {
        Oam[OamAddr] = value;
        OamAddr++;
    }

    private void WritePpuCtrl(byte value)
    {
        bool nmiWasEnabled = (PpuCtrl & CtrlNmi) != 0;
        PpuCtrl = value;
        ushort nt = (ushort)(value & CtrlBaseNtMask);
        T = (ushort)((T & (ushort)(NtSelectMask ^ 0xFFFF)) | (ushort)(nt << 10));
        if ((value & CtrlNmi) != 0 && InVblank())
        {
            if (Render.NmiRetrigger || !nmiWasEnabled) NmiRequest = true;
        }
    }

    private void WritePpuScroll(byte value)
    {
        if (!W)
        {
            T = (ushort)((T & 0xFFE0) | (value >> 3));
            FineX = (byte)(value & 0x07);
            W = true;
            Render.VDirty = true;
        }
        else
        {
            T = (ushort)((T & 0x8C1F) |
                          (ushort)(((value & 0xF8)) << 2) |
                          (ushort)(((value & 0x07)) << 12));
            W = false;
        }
    }

    private void WritePpuAddr(byte value)
    {
        if (!W)
        {
            T = (ushort)((T & 0x00FF) | (ushort)(((value & 0x3F)) << 8));
            W = true;
        }
        else
        {
            T = (ushort)((T & 0xFF00) | value);
            V = T;
            W = false;
        }
    }

    public ushort VramAddr() => V;
    public ushort VramIncrement() => ((PpuCtrl & CtrlIncrement32) != 0) ? (ushort)32 : (ushort)1;

    public void AdvanceVramAddr()
    {
        ushort inc = VramIncrement();
        V = (ushort)((V + inc) & 0x3FFF);
    }

    public byte PpuDataBufferValue() => PpuDataBuffer;
    public void SetPpuDataBuffer(byte value) => PpuDataBuffer = value;
    public byte OpenBusValue() => OpenBus;
    public void SetOpenBus(byte value) => OpenBus = value;

    private ushort MapNametable(ushort addr)
    {
        ushort a = (ushort)(addr & 0x2FFF);
        ushort local = (ushort)(a - 0x2000);
        ushort nt = (ushort)(local >> 10);
        ushort offset = (ushort)(local & 0x03FF);
        ushort phys = Mirroring switch
        {
            Mirroring.Horizontal => (ushort)(nt >> 1),
            Mirroring.Vertical => (ushort)(nt & 1),
            Mirroring.FourScreen => nt,
            Mirroring.SingleScreen0 => 0,
            Mirroring.SingleScreen1 => 1,
            Mirroring.SingleScreen2 => 2,
            Mirroring.SingleScreen3 => 3,
            _ => (ushort)(nt >> 1),
        };
        return (ushort)(phys * 0x400 + offset);
    }

    public byte ReadNametable(ushort addr) => Vram[MapNametable(addr)];
    public void WriteNametable(ushort addr, byte value) => Vram[MapNametable(addr)] = value;

    private static ushort MapPalette(ushort addr)
    {
        byte a = (byte)(addr & 0x1F);
        if ((a & 0x13) == 0x10) return (ushort)(a & 0x0F);
        return a;
    }

    public byte ReadPalette(ushort addr) => Palette[MapPalette(addr)];
    public void WritePalette(ushort addr, byte value) => Palette[MapPalette(addr)] = value;

    public void OamDma(byte[] data)
    {
        System.Array.Copy(data, 0, Oam, 0, 256);
        OamAddr = 0;
    }

    public void SetVblank(bool on)
    {
        if (on) PpuStatus |= StatusVblank;
        else PpuStatus &= (byte)(StatusVblank ^ 0xFF);
    }

    public void SetSpriteZeroHit(bool on)
    {
        if (on) PpuStatus |= StatusSpriteZero;
        else PpuStatus &= (byte)(StatusSpriteZero ^ 0xFF);
    }

    public void SetSpriteOverflow(bool on)
    {
        if (on) PpuStatus |= StatusOverflow;
        else PpuStatus &= (byte)(StatusOverflow ^ 0xFF);
    }

    public bool NmiEnabled() => (PpuCtrl & CtrlNmi) != 0;
    public bool InVblank() => (PpuStatus & StatusVblank) != 0;
    public bool SpriteZeroHit() => (PpuStatus & StatusSpriteZero) != 0;
    public bool SpriteOverflow() => (PpuStatus & StatusOverflow) != 0;

    public bool TakeNmiRequest()
    {
        bool r = NmiRequest;
        NmiRequest = false;
        return r;
    }

    private void IncrementHScroll()
    {
        if (Render.SlantCorruption)
        {
            if (FineX < 7)
            {
                FineX++;
                Render.FineXStart = (byte)((Render.FineXStart + 1) & 0x07);
            }
            else
            {
                FineX = 0;
                Render.FineXStart = 0;
                ushort coarseX = (ushort)(V & CoarseXMask);
                if (coarseX == 31)
                {
                    V &= (ushort)(CoarseXMask ^ 0xFFFF);
                    V ^= NtHBit;
                    Render.CoarseXStart = 0;
                    Render.NtHStart ^= NtHBit;
                }
                else
                {
                    V = (ushort)((V & (ushort)(CoarseXMask ^ 0xFFFF)) | (coarseX + 1));
                    Render.CoarseXStart = (ushort)((coarseX + 1) & CoarseXMask);
                }
            }
        }
        else
        {
            ushort coarseX = (ushort)(V & CoarseXMask);
            if (coarseX == 31)
            {
                V &= (ushort)(CoarseXMask ^ 0xFFFF);
                V ^= NtHBit;
            }
            else
            {
                V = (ushort)((V & (ushort)(CoarseXMask ^ 0xFFFF)) | (coarseX + 1));
            }
        }
    }

    private void IncrementVScroll()
    {
        ushort fineY = (ushort)((V & FineYMask) >> 12);
        if (fineY < 7)
        {
            V = (ushort)((V & (ushort)(FineYMask ^ 0xFFFF)) | ((fineY + 1) << 12));
        }
        else
        {
            V &= (ushort)(FineYMask ^ 0xFFFF);
            ushort coarseY = (ushort)((V & CoarseYMask) >> 5);
            if (coarseY == 29)
            {
                V &= (ushort)(CoarseYMask ^ 0xFFFF);
                V ^= NtVBit;
            }
            else if (coarseY == 31)
            {
                V &= (ushort)(CoarseYMask ^ 0xFFFF);
            }
            else
            {
                V = (ushort)((V & (ushort)(CoarseYMask ^ 0xFFFF)) | ((coarseY + 1) << 5));
            }
        }
    }

    private void CopyHTtoV()
    {
        ushort hBits = (ushort)(T & (CoarseXMask | NtHBit));
        V = (ushort)((V & (ushort)((CoarseXMask | NtHBit) ^ 0xFFFF)) | hBits);
    }

    private void CopyVTtoV()
    {
        ushort vBits = (ushort)(T & (CoarseYMask | NtVBit | FineYMask));
        V = (ushort)((V & (ushort)((CoarseYMask | NtVBit | FineYMask) ^ 0xFFFF)) | vBits);
    }

    public bool Step()
    {
        bool nmi = false;
        ushort scanlinesPerFrame = RegionTiming.ScanlinesPerFrame(Region);
        ushort prerender = RegionTiming.ScanlinePrerender(Region);

        Cycle++;
        if (Cycle >= CyclesPerScanline)
        {
            Cycle = 0;
            Scanline++;
            if (Scanline >= scanlinesPerFrame) Scanline = 0;
        }

        if (Scanline == ScanlineVblankStart && Cycle == VblankNmiCycle)
        {
            SetVblank(true);
            if (NmiEnabled())
            {
                NmiRequest = true;
                nmi = true;
            }
        }
        else if (Scanline == prerender && Cycle == VblankNmiCycle)
        {
            SetVblank(false);
            SetSpriteOverflow(false);
            SetSpriteZeroHit(false);
            Render.SpriteZeroHit = false;
        }

        bool rendering = (PpuMask & (MaskShowBg | MaskShowSprites)) != 0;
        if (rendering)
        {
            bool doesScrollInc = (Scanline < ScreenHeight) || (Scanline == prerender);
            if (doesScrollInc &&
                Cycle >= HScrollIncStep &&
                Cycle <= HScrollIncLast &&
                (Cycle % HScrollIncStep) == 0)
            {
                IncrementHScroll();
            }
            if (doesScrollInc && Cycle == VertScrollIncCycle) IncrementVScroll();
            if (doesScrollInc && Cycle == HCopyCycle) CopyHTtoV();
            if (Scanline == prerender &&
                Cycle >= VCopyCycleStart &&
                Cycle <= VCopyCycleEnd)
            {
                CopyVTtoV();
            }
        }

        return nmi;
    }

    public bool StepRendered(ChrReader chrRead)
    {
        bool nmi = Step();
        if (Cycle == 0)
        {
            Render.ScanlineInitialized = false;
            Render.PipelinePrimed = false;
        }
        if (Scanline < ScreenHeight && Cycle >= 1 && Cycle <= ScreenWidth)
        {
            PpuRender.RenderOnePixel(this, chrRead);
            Render.RenderedThisFrame = true;
        }
        return nmi;
    }

    public bool IsRendering() => (PpuMask & (MaskShowBg | MaskShowSprites)) != 0;
    public bool RenderedThisFrame() => Render.RenderedThisFrame;
    public void ResetRenderedFlag() => Render.RenderedThisFrame = false;
}
