package nes.core;

// NES joypad state for two standard controllers (port of cores/csharp/src/Joypad.cs).
public final class Joypad {
    public static final int CONTROLLER_COUNT = 2;
    public static final int BUTTON_COUNT = 8;

    public static final int BUTTON_A = 0;
    public static final int BUTTON_B = 1;
    public static final int BUTTON_SELECT = 2;
    public static final int BUTTON_START = 3;
    public static final int BUTTON_UP = 4;
    public static final int BUTTON_DOWN = 5;
    public static final int BUTTON_LEFT = 6;
    public static final int BUTTON_RIGHT = 7;

    public final byte[] current = new byte[CONTROLLER_COUNT];
    public boolean strobe;
    public final byte[] shift = new byte[CONTROLLER_COUNT];
    public final byte[] counter = new byte[CONTROLLER_COUNT];

    public Joypad() {}

    public void setButton(int controller, int button, boolean pressed) {
        if (controller < 0 || controller >= CONTROLLER_COUNT || button < 0 || button >= BUTTON_COUNT)
            return;
        int mask = 1 << button;
        if (pressed) current[controller] |= mask;
        else current[controller] &= ~mask;
        if (strobe) shift[controller] = current[controller];
    }

    public void writeStrobe(int value) {
        boolean newStrobe = (value & 0x01) != 0;
        if (strobe && !newStrobe) {
            for (int c = 0; c < CONTROLLER_COUNT; ++c) {
                shift[c] = current[c];
                counter[c] = 0;
            }
        }
        strobe = newStrobe;
        if (strobe) {
            for (int c = 0; c < CONTROLLER_COUNT; ++c)
                shift[c] = current[c];
        }
    }

    public int read(int controller) {
        if (controller < 0 || controller >= CONTROLLER_COUNT)
            return 1;
        if (strobe)
            return current[controller] & 0x01;
        int bit;
        if ((counter[controller] & 0xFF) < BUTTON_COUNT)
            bit = (shift[controller] >> (counter[controller] & 0xFF)) & 0x01;
        else
            bit = 1;
        if ((counter[controller] & 0xFF) < 0xFF)
            counter[controller] = (byte)((counter[controller] & 0xFF) + 1);
        return bit;
    }

    public void clear() {
        for (int c = 0; c < CONTROLLER_COUNT; ++c) {
            current[c] = 0;
            if (strobe) shift[c] = 0;
        }
    }
}
