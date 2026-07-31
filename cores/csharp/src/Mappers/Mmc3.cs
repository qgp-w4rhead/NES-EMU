using System.Runtime.CompilerServices;

namespace NesCore;

// Mapper 4 (MMC3). Port of src/mappers/mmc3.rs.
public sealed class Mmc3 : Mapper
{
    private const uint PrgBankSize = 8192;
    private const uint Chr1kSize = 1024;
    private const uint Chr2kSize = 2048;
    private const uint PrgRamSize = 8192;

    private byte[] _prgRom;
    private uint _prgSize;
    private byte[] _chr;
    private uint _chrSize;
    private bool _chrIsRam;
    private byte[] _prgRam = new byte[PrgRamSize];
    private bool _hasBattery;
    private byte _bankSelect;
    private byte[] _bankValues = new byte[8];
    private Mirroring _mirroring;
    private bool _prgRamEnable;
    private bool _prgRamWriteProtect;
    private byte _irqLatch;
    private byte _irqCounter;
    private bool _irqReloadFlag;
    private bool _irqEnable;
    private bool _irqPending;

    public Mmc3(byte[] prg, uint prgSize, byte[] chr, uint chrSize, Mirroring mirroring, bool hasBattery)
    {
        MapperNum = 4;
        _prgSize = prgSize;
        _prgRom = new byte[prgSize];
        if (prgSize > 0) System.Array.Copy(prg, _prgRom, (int)prgSize);
        if (chrSize == 0)
        {
            _chrSize = Chr2kSize * 4;
            _chr = new byte[_chrSize];
            _chrIsRam = true;
        }
        else
        {
            _chrSize = chrSize;
            _chr = new byte[chrSize];
            System.Array.Copy(chr, _chr, (int)chrSize);
            _chrIsRam = false;
        }
        _hasBattery = hasBattery;
        _bankSelect = 0;
        _mirroring = mirroring;
        _prgRamEnable = false;
        _prgRamWriteProtect = false;
        _irqLatch = 0;
        _irqCounter = 0;
        _irqReloadFlag = false;
        _irqEnable = false;
        _irqPending = false;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint PrgBankCount() => _prgSize / PrgBankSize > 0 ? _prgSize / PrgBankSize : 1;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint Chr1kCount() => _chrSize / Chr1kSize > 0 ? _chrSize / Chr1kSize : 1;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte PrgMode() => (byte)((_bankSelect >> 6) & 1);
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte ChrMode() => (byte)((_bankSelect >> 7) & 1);

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte PrgReadBank(uint bank, uint offset)
    {
        uint count = PrgBankCount();
        bank %= count;
        uint idx = bank * PrgBankSize + offset;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx];
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint ChrBankForSlot(uint slot)
    {
        byte[] r = _bankValues;
        byte cm = ChrMode();
        if (cm == 0)
        {
            return slot switch
            {
                0 => (uint)(r[0] & 0xFE),
                1 => (uint)(r[0] & 0xFE) + 1,
                2 => (uint)(r[1] & 0xFE),
                3 => (uint)(r[1] & 0xFE) + 1,
                4 => r[2],
                5 => r[3],
                6 => r[4],
                7 => r[5],
                _ => 0,
            };
        }
        else
        {
            return slot switch
            {
                0 => r[2],
                1 => r[3],
                2 => r[4],
                3 => r[5],
                4 => (uint)(r[0] & 0xFE),
                5 => (uint)(r[0] & 0xFE) + 1,
                6 => (uint)(r[1] & 0xFE),
                7 => (uint)(r[1] & 0xFE) + 1,
                _ => 0,
            };
        }
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint ChrIndex(ushort addr)
    {
        uint a = addr;
        uint slot = a / Chr1kSize;
        uint offset = a & (Chr1kSize - 1);
        uint bank = ChrBankForSlot(slot);
        uint count = Chr1kCount();
        return (bank % count) * Chr1kSize + offset;
    }

    public override byte ReadPrg(ushort addr)
    {
        if (addr >= 0x6000 && addr < 0x8000)
        {
            if (_prgRamEnable)
                return _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)];
            return 0x00;
        }
        uint local = (uint)(addr - 0x8000);
        uint slot = local / PrgBankSize;
        uint offset = local & (PrgBankSize - 1);
        uint count = PrgBankCount();
        uint last = count - 1;
        uint secondLast = count >= 2 ? count - 2 : 0;
        uint bank;
        byte pm = PrgMode();
        if (slot == 0)
            bank = (pm == 0) ? (_bankValues[6] % count) : secondLast;
        else if (slot == 1)
            bank = _bankValues[7] % count;
        else if (slot == 2)
            bank = (pm == 0) ? secondLast : (_bankValues[6] % count);
        else
            bank = last;
        return PrgReadBank(bank, offset);
    }

    public override void WritePrg(ushort addr, byte value)
    {
        if (addr >= 0x6000 && addr < 0x8000)
        {
            if (_prgRamEnable && !_prgRamWriteProtect)
                _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)] = value;
            return;
        }
        if (addr >= 0x8000 && addr <= 0x9FFF)
        {
            if ((addr & 1) == 0) _bankSelect = value;
            else { uint reg = (uint)(_bankSelect & 0x07); _bankValues[reg] = value; }
        }
        else if (addr >= 0xA000 && addr <= 0xBFFF)
        {
            if ((addr & 1) == 0) _mirroring = (value & 1) == 0 ? Mirroring.Vertical : Mirroring.Horizontal;
            else { _prgRamEnable = (value & 0x80) != 0; _prgRamWriteProtect = (value & 0x40) != 0; }
        }
        else if (addr >= 0xC000 && addr <= 0xDFFF)
        {
            if ((addr & 1) == 0) _irqLatch = value;
            else _irqReloadFlag = true;
        }
        else if (addr >= 0xE000 && addr <= 0xFFFF)
        {
            if ((addr & 1) == 0) { _irqEnable = false; _irqPending = false; }
            else _irqEnable = true;
        }
    }

    public override byte ReadChr(ushort addr)
    {
        uint idx = ChrIndex(addr);
        if (idx >= _chrSize) return 0x00;
        return _chr[idx];
    }

    public override void WriteChr(ushort addr, byte value)
    {
        if (!_chrIsRam) return;
        uint idx = ChrIndex(addr);
        if (idx < _chrSize) _chr[idx] = value;
    }

    public override Mirroring MirrorMode() => _mirroring;
    public override bool ChrIsRam() => _chrIsRam;
    public override bool HasBattery() => _hasBattery;
    public override bool IrqPending() => _irqPending;

    public override void ClockIrq()
    {
        if (_irqReloadFlag)
        {
            _irqCounter = _irqLatch;
            _irqReloadFlag = false;
        }
        else if (_irqCounter == 0)
        {
            _irqCounter = _irqLatch;
            if (_irqEnable) _irqPending = true;
        }
        else
        {
            _irqCounter--;
        }
    }

    public override uint SaveState(byte[] buf)
    {
        // Layout: bank_select(1) + bank_values(8) + mirroring(1) + prg_ram_enable(1)
        //         + prg_ram_write_protect(1) + irq_latch(1) + irq_counter(1)
        //         + irq_reload_flag(1) + irq_enable(1) + irq_pending(1) + prg_ram(8192) + chr(if ram)
        uint total = 17 + PrgRamSize + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = _bankSelect;
        for (int i = 0; i < 8; ++i) buf[p++] = _bankValues[i];
        buf[p++] = (byte)_mirroring;
        buf[p++] = (byte)(_prgRamEnable ? 1 : 0);
        buf[p++] = (byte)(_prgRamWriteProtect ? 1 : 0);
        buf[p++] = _irqLatch;
        buf[p++] = _irqCounter;
        buf[p++] = (byte)(_irqReloadFlag ? 1 : 0);
        buf[p++] = (byte)(_irqEnable ? 1 : 0);
        buf[p++] = (byte)(_irqPending ? 1 : 0);
        System.Array.Copy(_prgRam, 0, buf, p, (int)PrgRamSize); p += (int)PrgRamSize;
        if (_chrIsRam && _chrSize > 0)
        {
            System.Array.Copy(_chr, 0, buf, p, (int)_chrSize);
            p += (int)_chrSize;
        }
        return (uint)p;
    }

    public override bool LoadState(byte[] buf, uint len)
    {
        uint need = 17 + PrgRamSize + (_chrIsRam ? _chrSize : 0);
        if (len < need) return false;
        int p = 0;
        _bankSelect = buf[p++];
        for (int i = 0; i < 8; ++i) _bankValues[i] = buf[p++];
        _mirroring = (Mirroring)buf[p++];
        _prgRamEnable = buf[p++] != 0;
        _prgRamWriteProtect = buf[p++] != 0;
        _irqLatch = buf[p++];
        _irqCounter = buf[p++];
        _irqReloadFlag = buf[p++] != 0;
        _irqEnable = buf[p++] != 0;
        _irqPending = buf[p++] != 0;
        System.Array.Copy(buf, p, _prgRam, 0, (int)PrgRamSize); p += (int)PrgRamSize;
        if (_chrIsRam && _chrSize > 0)
        {
            System.Array.Copy(buf, p, _chr, 0, (int)_chrSize);
            p += (int)_chrSize;
        }
        return true;
    }
}
