package nes.core;

// Nametable mirroring mode (port of cores/csharp/src/Mapper.cs Mirroring).
public final class Mirroring {
    public static final int HORIZONTAL = 0;
    public static final int VERTICAL = 1;
    public static final int FOUR_SCREEN = 2;
    public static final int SINGLE_SCREEN0 = 3;
    public static final int SINGLE_SCREEN1 = 4;
    public static final int SINGLE_SCREEN2 = 5;
    public static final int SINGLE_SCREEN3 = 6;

    private Mirroring() {}
}
