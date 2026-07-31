// NES joypad state for two standard controllers (port of cores/c/src/joypad.c).

export class Joypad {
    static readonly ControllerCount = 2;
    static readonly ButtonCount = 8;

    current: Uint8Array = new Uint8Array(Joypad.ControllerCount);
    strobe: boolean = false;
    shift: Uint8Array = new Uint8Array(Joypad.ControllerCount);
    counter: Uint8Array = new Uint8Array(Joypad.ControllerCount);

    setButton(controller: number, button: number, pressed: boolean): void {
        if (controller < 0 || controller >= Joypad.ControllerCount || button < 0 || button >= Joypad.ButtonCount) return;
        const mask = 1 << button;
        if (pressed) this.current[controller] |= mask;
        else this.current[controller] &= (~mask & 0xFF);
        if (this.strobe) this.shift[controller] = this.current[controller];
    }

    writeStrobe(value: number): void {
        const newStrobe = (value & 0x01) !== 0;
        if (this.strobe && !newStrobe) {
            for (let c = 0; c < Joypad.ControllerCount; ++c) {
                this.shift[c] = this.current[c];
                this.counter[c] = 0;
            }
        }
        this.strobe = newStrobe;
        if (this.strobe) {
            for (let c = 0; c < Joypad.ControllerCount; ++c)
                this.shift[c] = this.current[c];
        }
    }

    read(controller: number): number {
        if (controller < 0 || controller >= Joypad.ControllerCount) return 1;
        if (this.strobe) return this.current[controller] & 0x01;
        let bit: number;
        if (this.counter[controller] < Joypad.ButtonCount)
            bit = (this.shift[controller] >> this.counter[controller]) & 0x01;
        else
            bit = 1;
        if (this.counter[controller] < 0xFF) this.counter[controller]++;
        return bit;
    }

    clear(): void {
        for (let c = 0; c < Joypad.ControllerCount; ++c) {
            this.current[c] = 0;
            if (this.strobe) this.shift[c] = 0;
        }
    }
}
