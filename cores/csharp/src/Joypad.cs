using System.Runtime.CompilerServices;

namespace NesCore;

// NES joypad state for two standard controllers (port of src/joypad.rs).
public sealed class Joypad
{
    public const int ControllerCount = 2;
    public const int ButtonCount = 8;

    // Button bit indices.
    public const int ButtonA = 0;
    public const int ButtonB = 1;
    public const int ButtonSelect = 2;
    public const int ButtonStart = 3;
    public const int ButtonUp = 4;
    public const int ButtonDown = 5;
    public const int ButtonLeft = 6;
    public const int ButtonRight = 7;

    public byte[] Current = new byte[ControllerCount];
    public bool Strobe;
    public byte[] Shift = new byte[ControllerCount];
    public byte[] Counter = new byte[ControllerCount];

    public Joypad()
    {
        // zeroed by default
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void SetButton(int controller, int button, bool pressed)
    {
        if (controller < 0 || controller >= ControllerCount || button < 0 || button >= ButtonCount)
            return;
        byte mask = (byte)(1u << button);
        if (pressed) Current[controller] |= mask;
        else Current[controller] &= (byte)~mask;
        if (Strobe) Shift[controller] = Current[controller];
    }

    public void WriteStrobe(byte value)
    {
        bool newStrobe = (value & 0x01) != 0;
        if (Strobe && !newStrobe)
        {
            for (int c = 0; c < ControllerCount; ++c)
            {
                Shift[c] = Current[c];
                Counter[c] = 0;
            }
        }
        Strobe = newStrobe;
        if (Strobe)
        {
            for (int c = 0; c < ControllerCount; ++c)
                Shift[c] = Current[c];
        }
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public byte Read(int controller)
    {
        if (controller < 0 || controller >= ControllerCount)
            return 1;
        if (Strobe)
            return (byte)(Current[controller] & 0x01);
        byte bit;
        if (Counter[controller] < ButtonCount)
            bit = (byte)((Shift[controller] >> Counter[controller]) & 0x01);
        else
            bit = 1;
        if (Counter[controller] < 0xFF)
            Counter[controller]++;
        return bit;
    }

    public void Clear()
    {
        for (int c = 0; c < ControllerCount; ++c)
        {
            Current[c] = 0;
            if (Strobe) Shift[c] = 0;
        }
    }
}
