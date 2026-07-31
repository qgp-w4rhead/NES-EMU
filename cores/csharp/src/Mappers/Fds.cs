using System.Runtime.CompilerServices;

namespace NesCore;

// Mapper 20 (Famicom Disk System RAM adapter + disk drive).
// Port of src/mappers/fds.rs.
public sealed class Fds : Mapper
{
    private const uint PrgRamSize = 32 * 1024;
    private const uint ChrRamSize = 8 * 1024;
    private const uint BiosSize = 8 * 1024;

    private byte[] _prgRam = new byte[PrgRamSize];
    private byte[] _bios;
    private byte[] _chrRam = new byte[ChrRamSize];
    private byte[] _diskData;
    private uint _diskSize;
    private uint _diskReadPos;
    private uint _diskWritePos;
    private bool _diskMotorOn;
    private bool _diskTransferReset;
    private bool _diskWriteMode;
    private bool _diskDataAvailable;
    private bool _diskInserted;
    private byte _diskWriteLatch;
    private bool _ioEnableDisk;
    private bool _ioEnableTimer;
    private ushort _timerLatch;
    private ushort _timerCounter;
    private bool _timerEnable;
    private bool _timerIrqPending;
    private bool _mirrorHorizontal;
    private FdsAudio _audio = new FdsAudio();

    public Fds(byte[] bios, int biosLen, byte[] diskData, uint diskLen)
    {
        MapperNum = 20;
        _bios = new byte[BiosSize];
        int copy = biosLen;
        if (copy > (int)BiosSize) copy = (int)BiosSize;
        if (copy > 0 && bios != null) System.Array.Copy(bios, _bios, copy);
        if (diskLen > 0 && diskData != null)
        {
            _diskData = new byte[diskLen];
            System.Array.Copy(diskData, _diskData, (int)diskLen);
            _diskSize = diskLen;
        }
        _diskInserted = true;
        _ioEnableDisk = true;
        _ioEnableTimer = true;
    }

    private void WriteDiskRegister(ushort addr, byte value)
    {
        switch (addr)
        {
            case 0x4020: _timerLatch = (ushort)((_timerLatch & 0xFF00) | value); break;
            case 0x4021: _timerLatch = (ushort)((_timerLatch & 0x00FF) | ((ushort)value << 8)); break;
            case 0x4022:
                _timerEnable = (value & 0x01) != 0;
                if (!_timerEnable) _timerIrqPending = false;
                if (_timerEnable) _timerCounter = _timerLatch;
                break;
            case 0x4023:
                _ioEnableDisk = (value & 0x01) != 0;
                _ioEnableTimer = (value & 0x02) != 0;
                if (!_ioEnableTimer) _timerIrqPending = false;
                break;
            case 0x4024:
                _diskWriteLatch = value;
                if (_diskWriteMode && _ioEnableDisk)
                {
                    if (_diskWritePos < _diskSize)
                    {
                        _diskData[_diskWritePos] = value;
                        _diskWritePos++;
                    }
                }
                break;
            case 0x4025:
                if (!_ioEnableDisk) return;
                bool reset = (value & 0x01) != 0;
                if (reset && !_diskTransferReset)
                {
                    _diskReadPos = 0;
                    _diskWritePos = 0;
                    _diskDataAvailable = false;
                }
                _diskTransferReset = reset;
                _mirrorHorizontal = (value & 0x02) != 0;
                _diskWriteMode = (value & 0x04) != 0;
                _diskMotorOn = (value & 0x20) != 0;
                if (_diskMotorOn && !_diskWriteMode)
                    _diskDataAvailable = _diskReadPos < _diskSize;
                break;
        }
    }

    private byte ReadDiskRegister(ushort addr)
    {
        switch (addr)
        {
            case 0x4030:
                {
                    byte status = 0;
                    if (_diskDataAvailable) status |= 0x01;
                    if (_diskMotorOn && _diskInserted) status |= 0x04;
                    if (_diskTransferReset) status |= 0x10;
                    if (_timerIrqPending) status |= 0x80;
                    _timerIrqPending = false;
                    return status;
                }
            case 0x4031:
                {
                    if (_diskReadPos < _diskSize)
                    {
                        byte b = _diskData[_diskReadPos];
                        _diskReadPos++;
                        _diskDataAvailable = _diskReadPos < _diskSize;
                        return b;
                    }
                    return 0x00;
                }
            case 0x4032: return _diskInserted ? (byte)0x00 : (byte)0x01;
            case 0x4033: return 0x80;
            default: return 0x00;
        }
    }

    public override byte ReadPrg(ushort addr)
    {
        if (addr >= 0x6000 && addr < 0xE000)
            return _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)];
        if (addr >= 0xE000)
            return _bios[(uint)(addr - 0xE000) & (BiosSize - 1)];
        return 0x00;
    }

    public override byte ReadPrgMut(ushort addr)
    {
        if (addr >= 0x6000 && addr < 0xE000)
            return _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)];
        if (addr >= 0xE000)
            return _bios[(uint)(addr - 0xE000) & (BiosSize - 1)];
        if (addr >= 0x4020 && addr <= 0x4033) return ReadDiskRegister(addr);
        if (addr == 0x4090 || addr == 0x4092) return _audio.ReadRegister(addr);
        return 0x00;
    }

    public override void WritePrg(ushort addr, byte value)
    {
        if (addr >= 0x6000 && addr < 0xE000)
        {
            _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)] = value;
            return;
        }
        if (addr >= 0x4020 && addr <= 0x4033) { WriteDiskRegister(addr, value); return; }
        if (addr >= 0x4040 && addr <= 0x408A) _audio.WriteRegister(addr, value);
    }

    public override byte ReadChr(ushort addr) => _chrRam[addr & (ChrRamSize - 1)];
    public override void WriteChr(ushort addr, byte value) => _chrRam[addr & (ChrRamSize - 1)] = value;

    public override Mirroring MirrorMode() => _mirrorHorizontal ? Mirroring.Horizontal : Mirroring.Vertical;
    public override bool ChrIsRam() => true;
    public override bool HasBattery() => false;
    public override bool IrqPending() => _timerIrqPending;

    public override void ClockCpu(uint cpuCycles)
    {
        if (_timerEnable && _ioEnableTimer)
        {
            for (uint i = 0; i < cpuCycles; ++i)
            {
                if (_timerCounter == 0)
                {
                    _timerCounter = _timerLatch;
                    _timerIrqPending = true;
                }
                else
                {
                    _timerCounter--;
                }
            }
        }
        _audio.Clock(cpuCycles / 2);
    }

    public override float ExpansionAudioSample() => _audio.Sample();
}
