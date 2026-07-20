//! Integration tests for the M29 screenshot PNG encoder.
//!
//! These exercise the *public* API of [`nes_emu::screenshot`]. Tests for
//! the private `adler32` / `crc32` / `zlib_stored` helpers live as unit
//! tests inside `src/screenshot.rs` (they need access to private items).

use nes_emu::screenshot;
use nes_emu::video::{NES_HEIGHT, NES_WIDTH};

/// PNG 8-byte signature.
const PNG_SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Decode the zlib stored-block stream inside an IDAT chunk back into
/// the raw scanline bytes (filter byte + RGB triples per row). Used to
/// verify round-trip integrity without an external PNG decoder.
fn decode_zlib_stored(z: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 2usize; // skip CMF + FLG
    let end = z.len() - 4; // drop Adler-32 trailer
    while i < end {
        let header = z[i];
        let bfinal = header & 1 != 0;
        i += 1;
        let len = u16::from_le_bytes([z[i], z[i + 1]]) as usize;
        i += 2;
        i += 2; // NLEN
        out.extend_from_slice(&z[i..i + len]);
        i += len;
        if bfinal {
            break;
        }
    }
    out
}

/// Parse a PNG byte stream into a list of (type, data) chunks, verifying
/// the CRC of each chunk along the way. Returns the chunks in file order.
fn parse_chunks(png: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    assert_eq!(&png[..8], &PNG_SIG, "PNG signature");
    let mut chunks = Vec::new();
    let mut i = 8usize;
    while i < png.len() {
        let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
        let chunk_type: [u8; 4] = png[i + 4..i + 8].try_into().unwrap();
        let data = &png[i + 8..i + 8 + len];
        let crc_stored = u32::from_be_bytes(png[i + 8 + len..i + 8 + len + 4].try_into().unwrap());
        // Recompute CRC over type + data and verify.
        let mut crc_input = Vec::with_capacity(4 + len);
        crc_input.extend_from_slice(&chunk_type);
        crc_input.extend_from_slice(data);
        let crc_computed = crc32_of(&crc_input);
        assert_eq!(
            crc_stored,
            crc_computed,
            "CRC mismatch on chunk {:?}",
            std::str::from_utf8(&chunk_type).unwrap_or("???")
        );
        chunks.push((chunk_type, data.to_vec()));
        i += 8 + len + 4;
        if &chunk_type == b"IEND" {
            break;
        }
    }
    chunks
}

/// Minimal CRC-32 (PNG/zlib polynomial 0xEDB88320) for test-side
/// verification — independent of the implementation under test.
fn crc32_of(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for n in 0..256u32 {
        let mut c = n;
        for _ in 0..8 {
            if c & 1 != 0 {
                c = 0xEDB88320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
        }
        table[n as usize] = c;
    }
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc = (crc >> 8) ^ table[((crc ^ b as u32) & 0xFF) as usize];
    }
    crc ^ 0xFFFF_FFFF
}

/// A 1x1 PNG must have the signature, exactly three chunks (IHDR, IDAT,
/// IEND), and IEND must be the last chunk.
#[test]
fn one_pixel_png_structure() {
    let png = screenshot::encode_to_bytes(&[0xFFFF_0000], 1, 1);
    assert_eq!(&png[..8], &PNG_SIG);
    let chunks = parse_chunks(&png);
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].0, *b"IHDR");
    assert_eq!(chunks[1].0, *b"IDAT");
    assert_eq!(chunks[2].0, *b"IEND");
}

/// IHDR must encode width, height, bit depth 8, color type 2 (RGB),
/// compression 0, filter 0, interlace 0.
#[test]
fn ihdr_fields_for_2x2() {
    let png =
        screenshot::encode_to_bytes(&[0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF, 0xFFFFFFFF], 2, 2);
    let chunks = parse_chunks(&png);
    let ihdr = &chunks[0].1;
    assert_eq!(ihdr.len(), 13);
    assert_eq!(&ihdr[0..4], &2u32.to_be_bytes(), "width");
    assert_eq!(&ihdr[4..8], &2u32.to_be_bytes(), "height");
    assert_eq!(ihdr[8], 8, "bit depth");
    assert_eq!(ihdr[9], 2, "color type RGB");
    assert_eq!(ihdr[10], 0, "compression");
    assert_eq!(ihdr[11], 0, "filter");
    assert_eq!(ihdr[12], 0, "interlace");
}

/// The zlib stream inside IDAT must use CMF=0x78 and a FLG satisfying
/// the (CMF*256 + FLG) % 31 == 0 check value (RFC 1950).
#[test]
fn idat_zlib_header_check_value() {
    let png = screenshot::encode_to_bytes(&[0xFFFF_0000], 1, 1);
    let chunks = parse_chunks(&png);
    let idat = &chunks[1].1;
    let cmf = idat[0];
    let flg = idat[1];
    assert_eq!(cmf, 0x78);
    assert_eq!(((cmf as u32) * 256 + flg as u32) % 31, 0);
}

/// Decoding the IDAT zlib stream of a 2x2 PNG must yield the expected
/// raw scanline bytes: one filter byte (0x00) per row followed by RGB
/// triples in pixel order.
#[test]
fn encode_2x2_round_trips_pixels() {
    let fb: [u32; 4] = [0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF, 0xFFFFFFFF];
    let png = screenshot::encode_to_bytes(&fb, 2, 2);
    let chunks = parse_chunks(&png);
    let decoded = decode_zlib_stored(&chunks[1].1);
    let expected = [
        0, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, // row 0: red, green
        0, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, // row 1: blue, white
    ];
    assert_eq!(decoded, expected);
}

/// `encode_to_path` writes a file whose bytes match `encode_to_bytes`.
#[test]
fn encode_to_path_writes_file() {
    let dir = std::env::temp_dir().join(format!("nes-emu-shot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shot.png");
    let _ = std::fs::remove_file(&path);

    let fb: [u32; 4] = [0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF, 0xFFFFFFFF];
    screenshot::encode_to_path(&fb, 2, 2, &path).expect("write");
    let on_disk = std::fs::read(&path).expect("read");
    assert_eq!(on_disk, screenshot::encode_to_bytes(&fb, 2, 2));

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
}

/// `encode_to_path` creates parent directories that do not yet exist.
#[test]
fn encode_to_path_creates_parent_dirs() {
    let dir = std::env::temp_dir()
        .join(format!("nes-emu-shot-nested-{}", std::process::id()))
        .join("sub");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    let path = dir.join("deep.png");
    screenshot::encode_to_path(&[0xFF00_0000], 1, 1, &path).expect("write");
    assert!(path.exists());
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// A full 256x240 framebuffer (the actual NES resolution) must encode
/// without crashing, produce a valid PNG structure, and round-trip the
/// pixel data through the zlib stored-block stream.
#[test]
fn full_nes_framebuffer_encodes_and_round_trips() {
    let fb = vec![0xFFFF_0000; (NES_WIDTH * NES_HEIGHT) as usize];
    let png = screenshot::encode_to_bytes(&fb, NES_WIDTH, NES_HEIGHT);
    let chunks = parse_chunks(&png);
    assert_eq!(chunks.len(), 3);
    // IHDR width/height.
    let ihdr = &chunks[0].1;
    assert_eq!(&ihdr[0..4], &NES_WIDTH.to_be_bytes());
    assert_eq!(&ihdr[4..8], &NES_HEIGHT.to_be_bytes());
    // Decoded raw = 240 rows * (1 filter byte + 256*3 RGB) = 184560.
    let decoded = decode_zlib_stored(&chunks[1].1);
    assert_eq!(decoded.len(), 240 * (1 + 256 * 3));
    // Every byte after each filter byte should be R=0xFF, G=0x00, B=0x00.
    for row in 0..240 {
        let off = row * (1 + 256 * 3);
        assert_eq!(decoded[off], 0, "filter byte for row {row}");
        for px in 0..256 {
            let p = off + 1 + px * 3;
            assert_eq!(decoded[p], 0xFF, "R at row {row} px {px}");
            assert_eq!(decoded[p + 1], 0x00, "G at row {row} px {px}");
            assert_eq!(decoded[p + 2], 0x00, "B at row {row} px {px}");
        }
    }
}

/// A large framebuffer (>65535 raw bytes) must split across multiple
/// stored deflate blocks. The IDAT length must reflect the extra block
/// headers (5 bytes per 65535-byte chunk).
#[test]
fn large_framebuffer_splits_into_multiple_blocks() {
    let fb = vec![0xFFFF_0000; (NES_WIDTH * NES_HEIGHT) as usize];
    let png = screenshot::encode_to_bytes(&fb, NES_WIDTH, NES_HEIGHT);
    let chunks = parse_chunks(&png);
    let idat_len = chunks[1].1.len();
    // raw = 184560, blocks = ceil(184560/65535) = 3.
    // idat_len >= 2 (zlib hdr) + 3*5 (block hdrs) + 184560 + 4 (adler).
    assert!(idat_len >= 184_581, "idat_len = {idat_len}");
}

/// Encoding the same framebuffer twice must produce byte-identical
/// output (deterministic — no time- or pointer-derived data in the
/// PNG bytes).
#[test]
fn encoding_is_deterministic() {
    let fb: [u32; 4] = [0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF, 0xFFFFFFFF];
    let a = screenshot::encode_to_bytes(&fb, 2, 2);
    let b = screenshot::encode_to_bytes(&fb, 2, 2);
    assert_eq!(a, b);
}

/// A framebuffer with distinct per-pixel colors must round-trip every
/// pixel through the PNG → zlib → raw scanline path.
#[test]
fn gradient_framebuffer_round_trips_all_pixels() {
    let w = 16u32;
    let h = 4u32;
    let fb: Vec<u32> = (0..(w * h)).map(|i| 0xFF00_0000 | i).collect();
    let png = screenshot::encode_to_bytes(&fb, w, h);
    let chunks = parse_chunks(&png);
    let decoded = decode_zlib_stored(&chunks[1].1);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let off = y as usize * (1 + w as usize * 3) + 1 + x as usize * 3;
            let px = fb[i];
            assert_eq!(decoded[off], ((px >> 16) & 0xFF) as u8, "R at ({x},{y})");
            assert_eq!(decoded[off + 1], ((px >> 8) & 0xFF) as u8, "G at ({x},{y})");
            assert_eq!(decoded[off + 2], (px & 0xFF) as u8, "B at ({x},{y})");
        }
    }
}
