//! Hand-written Gzip and ZIP decompression for ROM archives (no external crates).
//!
//! Supports DEFLATE (RFC 1951), Gzip (RFC 1952), and ZIP formats.

use std::fmt;

/// Errors that can occur during decompression.
#[derive(Debug)]
pub enum DecompressError {
    /// The input was too short for the format's header.
    TooShort,
    /// The input did not match the expected magic / signature.
    BadMagic,
    /// The DEFLATE stream contained an invalid or unsupported block type.
    InvalidBlockType,
    /// A stored block's length / complement length check failed.
    StoredLenMismatch,
    /// The DEFLATE bit stream ran out of bits mid-symbol.
    UnexpectedEof,
    /// A Huffman code length table was invalid (over- or under-subscribed).
    InvalidHuffman,
    /// A symbol was outside the valid range for its alphabet.
    InvalidSymbol,
    /// A ZIP archive contained no usable file entry.
    NoFileInZip,
    /// A ZIP entry used an unsupported compression method.
    UnsupportedZipMethod(u16),
    /// A Gzip header flag indicated an unsupported feature.
    UnsupportedGzipFlag,
    /// A Gzip CRC32 / size check failed.
    GzipChecksumMismatch,
    /// A generic decode failure with a descriptive message.
    Other(String),
}

impl fmt::Display for DecompressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecompressError::TooShort => write!(f, "compressed input too short"),
            DecompressError::BadMagic => write!(f, "bad magic / signature"),
            DecompressError::InvalidBlockType => write!(f, "invalid DEFLATE block type"),
            DecompressError::StoredLenMismatch => write!(f, "stored block length mismatch"),
            DecompressError::UnexpectedEof => write!(f, "unexpected end of DEFLATE stream"),
            DecompressError::InvalidHuffman => write!(f, "invalid Huffman code lengths"),
            DecompressError::InvalidSymbol => write!(f, "invalid DEFLATE symbol"),
            DecompressError::NoFileInZip => write!(f, "ZIP archive contains no usable file"),
            DecompressError::UnsupportedZipMethod(m) => {
                write!(f, "unsupported ZIP compression method {m}")
            }
            DecompressError::UnsupportedGzipFlag => write!(f, "unsupported Gzip header flag"),
            DecompressError::GzipChecksumMismatch => write!(f, "Gzip CRC/size check failed"),
            DecompressError::Other(m) => write!(f, "decompression error: {m}"),
        }
    }
}

impl std::error::Error for DecompressError {}

// ---------------------------------------------------------------------------
// DEFLATE (RFC 1951) — bit reader + Huffman decoding
// ---------------------------------------------------------------------------

/// Bit reader for DEFLATE streams. DEFLATE packs bits LSB-first within
/// each byte; Huffman codes are packed MSB-first (we reverse them when
/// building the decode table).
struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    /// Bit buffer (bits packed in from the low end).
    bit_buf: u32,
    /// Number of valid bits currently in `bit_buf`.
    bit_count: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader {
            data,
            byte_pos: 0,
            bit_buf: 0,
            bit_count: 0,
        }
    }

    /// Ensure at least `n` bits are available in the buffer (n ≤ 24).
    fn fill(&mut self, n: u32) -> Result<(), DecompressError> {
        while self.bit_count < n {
            if self.byte_pos >= self.data.len() {
                return Err(DecompressError::UnexpectedEof);
            }
            self.bit_buf |= (self.data[self.byte_pos] as u32) << self.bit_count;
            self.bit_count += 8;
            self.byte_pos += 1;
        }
        Ok(())
    }

    /// Read `n` bits (LSB-first) from the stream. `n` must be ≤ 24.
    fn read_bits(&mut self, n: u32) -> Result<u32, DecompressError> {
        if n == 0 {
            return Ok(0);
        }
        self.fill(n)?;
        let value = self.bit_buf & ((1u32 << n) - 1);
        self.bit_buf >>= n;
        self.bit_count -= n;
        Ok(value)
    }

    /// Drop any remaining bits in the current partial byte, advancing to
    /// the next byte boundary. Used by stored blocks.
    fn align_to_byte(&mut self) {
        let drop = self.bit_count & 7;
        self.bit_buf >>= drop;
        self.bit_count -= drop;
        // Flush whole buffered bytes back into the position counter so
        // `read_byte` can pull them in order.
        let buffered_bytes = self.bit_count / 8;
        self.byte_pos -= buffered_bytes as usize;
        self.bit_buf = 0;
        self.bit_count = 0;
    }

    /// Read one literal byte after `align_to_byte`. Used by stored blocks.
    fn read_byte(&mut self) -> Result<u8, DecompressError> {
        if self.byte_pos >= self.data.len() {
            return Err(DecompressError::UnexpectedEof);
        }
        let b = self.data[self.byte_pos];
        self.byte_pos += 1;
        Ok(b)
    }
}

/// A simple canonical-Huffman decoder. Codes are stored as `(code, length,
/// symbol)` triples sorted by length then value, and matched bit-by-bit
/// from the MSB (Huffman codes are packed MSB-first inside the LSB-first
/// DEFLATE bit stream — we read them by reversing the bit order).
struct HuffmanTable {
    /// (code_length, code_value, symbol) sorted by length ascending then
    /// code value ascending.
    codes: Vec<(u32, u32, u16)>,
}

impl HuffmanTable {
    /// Build a canonical Huffman table from per-symbol code lengths.
    /// `lengths[i]` is the bit length of the code for symbol `i`; a length
    /// of 0 means the symbol is unused. Returns an error if the lengths
    /// are over- or under-subscribed (i.e. do not form a complete prefix
    /// code).
    fn from_lengths(lengths: &[u8]) -> Result<Self, DecompressError> {
        let max_len = lengths.iter().copied().max().unwrap_or(0) as usize;
        if max_len == 0 {
            return Ok(HuffmanTable { codes: Vec::new() });
        }
        // Count codes per length.
        let mut bl_count = vec![0u32; max_len + 1];
        for &l in lengths {
            if l > 0 {
                bl_count[l as usize] += 1;
            }
        }
        // Compute the first code value for each length (canonical codes).
        let mut next_code = vec![0u32; max_len + 1];
        let mut code: u32 = 0;
        for bits in 1..=max_len {
            code = (code + bl_count[bits - 1]) << 1;
            next_code[bits] = code;
        }
        // Assign codes to symbols in symbol order.
        let mut codes: Vec<(u32, u32, u16)> = Vec::new();
        for (sym, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let c = next_code[len as usize];
            next_code[len as usize] += 1;
            codes.push((len as u32, c, sym as u16));
        }
        // Validate completeness: the Kraft inequality must be exactly
        // satisfied for a complete code (sum of 2^-len == 1). We allow
        // incomplete codes only when there is exactly one symbol with a
        // 1-bit code (a degenerate but legal case used by some encoders
        // for single-distance tables).
        let mut kraft: u64 = 0;
        for &(len, _, _) in &codes {
            kraft += 1u64 << (max_len - len as usize);
        }
        let full = 1u64 << max_len;
        if kraft != full {
            // Special case: a single code of length 1 is legal (RFC 1951
            // section 3.2.7 allows it for distance codes with one symbol).
            if !(codes.len() == 1 && codes[0].0 == 1) {
                return Err(DecompressError::InvalidHuffman);
            }
        }
        // Sort by length ascending then code value ascending for the
        // greedy matcher.
        codes.sort_by_key(|&(len, code, _)| (len, code));
        Ok(HuffmanTable { codes })
    }

    /// Decode one symbol from the bit reader. Reads bits MSB-first within
    /// the Huffman code (so we accumulate bits into the high end of the
    /// code value).
    fn decode(&self, br: &mut BitReader<'_>) -> Result<u16, DecompressError> {
        if self.codes.is_empty() {
            return Err(DecompressError::InvalidHuffman);
        }
        let mut code: u32 = 0;
        let mut len: u32 = 0;
        loop {
            len += 1;
            let bit = br.read_bits(1)?;
            code = (code << 1) | bit;
            // Find a matching code of the current length. Since codes are
            // sorted by (length, code), we can scan linearly — DEFLATE
            // codes are at most 15 bits, and the table is small.
            for &(clen, ccode, sym) in &self.codes {
                if clen == len && ccode == code {
                    return Ok(sym);
                }
            }
            if len > 15 {
                return Err(DecompressError::InvalidSymbol);
            }
        }
    }
}

/// Length code table (RFC 1951 section 3.2.5). `LENGTH_BASE[c - 257]` is
/// the base length and `LENGTH_EXTRA[c - 257]` is the number of extra
/// bits to read.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// Distance code table (RFC 1951 section 3.2.5). `DIST_BASE[c]` is the
/// base distance and `DIST_EXTRA[c]` is the number of extra bits.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Order in which code-length-code lengths appear in a dynamic block
/// (RFC 1951 section 3.2.7).
const CL_ORDER: [u8; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Build the fixed Huffman tables specified in RFC 1951 section 3.2.6.
fn fixed_litlen_table() -> HuffmanTable {
    let mut lengths = [0u8; 288];
    for b in &mut lengths[0..=143] {
        *b = 8;
    }
    for b in &mut lengths[144..=255] {
        *b = 9;
    }
    for b in &mut lengths[256..=279] {
        *b = 7;
    }
    for b in &mut lengths[280..=287] {
        *b = 8;
    }
    HuffmanTable::from_lengths(&lengths).expect("fixed lit/len lengths are valid")
}

fn fixed_dist_table() -> HuffmanTable {
    // RFC 1951 section 3.2.6: distance codes 0-31 are all 5 bits.
    let lengths = [5u8; 32];
    HuffmanTable::from_lengths(&lengths).expect("fixed distance lengths are valid")
}

/// Inflate (decompress) a raw DEFLATE stream into a new `Vec<u8>`.
pub fn inflate(data: &[u8]) -> Result<Vec<u8>, DecompressError> {
    let mut br = BitReader::new(data);
    let mut out: Vec<u8> = Vec::new();
    loop {
        let bfinal = br.read_bits(1)?;
        let btype = br.read_bits(2)?;
        match btype {
            0 => {
                // Stored block: align to byte, read LEN + NLEN + data.
                br.align_to_byte();
                let lo = br.read_byte()? as u16;
                let hi = br.read_byte()? as u16;
                let len = (lo | (hi << 8)) as usize;
                let nlo = br.read_byte()? as u16;
                let nhi = br.read_byte()? as u16;
                let nlen = nlo | (nhi << 8);
                if len != (!nlen) as usize {
                    return Err(DecompressError::StoredLenMismatch);
                }
                for _ in 0..len {
                    out.push(br.read_byte()?);
                }
            }
            1 => {
                let lit = fixed_litlen_table();
                let dist = fixed_dist_table();
                decode_block(&mut br, &mut out, &lit, &dist)?;
            }
            2 => {
                let (lit, dist) = decode_dynamic_tables(&mut br)?;
                decode_block(&mut br, &mut out, &lit, &dist)?;
            }
            _ => return Err(DecompressError::InvalidBlockType),
        }
        if bfinal != 0 {
            break;
        }
    }
    Ok(out)
}

/// Read the dynamic Huffman tables from the start of a BTYPE=10 block.
fn decode_dynamic_tables(
    br: &mut BitReader<'_>,
) -> Result<(HuffmanTable, HuffmanTable), DecompressError> {
    let hlit = br.read_bits(5)? as usize + 257;
    let hdist = br.read_bits(5)? as usize + 1;
    let hclen = br.read_bits(4)? as usize + 4;

    let mut cl_lengths = [0u8; 19];
    for i in 0..hclen {
        cl_lengths[CL_ORDER[i] as usize] = br.read_bits(3)? as u8;
    }
    let cl_table = HuffmanTable::from_lengths(&cl_lengths)?;

    // Decode the combined lit/len + distance code lengths using the
    // code-length Huffman code.
    let total = hlit + hdist;
    let mut all_lengths = Vec::with_capacity(total);
    while all_lengths.len() < total {
        let sym = cl_table.decode(br)?;
        match sym {
            0..=15 => all_lengths.push(sym as u8),
            16 => {
                // Copy the previous code length 3-6 times.
                if all_lengths.is_empty() {
                    return Err(DecompressError::InvalidHuffman);
                }
                let prev = *all_lengths.last().unwrap();
                let repeat = br.read_bits(2)? as usize + 3;
                for _ in 0..repeat {
                    all_lengths.push(prev);
                }
            }
            17 => {
                // Repeat zero 3-10 times.
                let repeat = br.read_bits(3)? as usize + 3;
                all_lengths.extend(std::iter::repeat(0).take(repeat));
            }
            18 => {
                // Repeat zero 11-138 times.
                let repeat = br.read_bits(7)? as usize + 11;
                all_lengths.extend(std::iter::repeat(0).take(repeat));
            }
            _ => return Err(DecompressError::InvalidSymbol),
        }
    }
    if all_lengths.len() != total {
        return Err(DecompressError::InvalidHuffman);
    }
    let lit_lengths = &all_lengths[..hlit];
    let dist_lengths = &all_lengths[hlit..];
    let lit = HuffmanTable::from_lengths(lit_lengths)?;
    let dist = HuffmanTable::from_lengths(dist_lengths)?;
    Ok((lit, dist))
}

/// Decode the body of a Huffman-coded block (BTYPE=01 or 10) into `out`.
fn decode_block(
    br: &mut BitReader<'_>,
    out: &mut Vec<u8>,
    lit: &HuffmanTable,
    dist: &HuffmanTable,
) -> Result<(), DecompressError> {
    loop {
        let sym = lit.decode(br)?;
        match sym {
            256 => return Ok(()), // End of block.
            s if s < 256 => out.push(s as u8),
            _ => {
                // Length/distance pair.
                let li = (sym - 257) as usize;
                if li >= LENGTH_BASE.len() {
                    return Err(DecompressError::InvalidSymbol);
                }
                let mut length = LENGTH_BASE[li] as usize;
                let lextra = LENGTH_EXTRA[li];
                if lextra > 0 {
                    length += br.read_bits(lextra as u32)? as usize;
                }
                let dsym = dist.decode(br)? as usize;
                if dsym >= DIST_BASE.len() {
                    return Err(DecompressError::InvalidSymbol);
                }
                let mut distance = DIST_BASE[dsym] as usize;
                let dextra = DIST_EXTRA[dsym];
                if dextra > 0 {
                    distance += br.read_bits(dextra as u32)? as usize;
                }
                if distance == 0 || distance > out.len() {
                    return Err(DecompressError::InvalidSymbol);
                }
                // Copy `length` bytes from `distance` back in the output.
                let start = out.len() - distance;
                for i in 0..length {
                    let b = out[start + i];
                    out.push(b);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Gzip (RFC 1952)
// ---------------------------------------------------------------------------

/// Gzip magic bytes: `0x1F 0x8B`.
const GZIP_MAGIC: [u8; 2] = [0x1F, 0x8B];
/// Gzip compression method for DEFLATE.
const GZIP_METHOD_DEFLATE: u8 = 8;
/// Gzip flag bits.
const GZIP_FTEXT: u8 = 0x01;
const GZIP_FHCRC: u8 = 0x02;
const GZIP_FEXTRA: u8 = 0x04;
const GZIP_FNAME: u8 = 0x08;
const GZIP_FCOMMENT: u8 = 0x10;

/// Decompress a Gzip stream and return the payload. The trailing CRC32
/// and ISIZE fields are checked against the decompressed output.
pub fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, DecompressError> {
    if data.len() < 18 {
        return Err(DecompressError::TooShort);
    }
    if data[0] != GZIP_MAGIC[0] || data[1] != GZIP_MAGIC[1] {
        return Err(DecompressError::BadMagic);
    }
    let cm = data[2];
    if cm != GZIP_METHOD_DEFLATE {
        return Err(DecompressError::Other(format!(
            "unsupported gzip compression method {cm}"
        )));
    }
    let flg = data[3];
    if (flg & !0x1F) != 0 {
        // Reserved bits set — reject.
        return Err(DecompressError::UnsupportedGzipFlag);
    }
    // MTIME(4) + XFL(1) + OS(1) = 6 bytes after flags.
    let mut pos = 10;
    if flg & GZIP_FEXTRA != 0 {
        if pos + 2 > data.len() {
            return Err(DecompressError::TooShort);
        }
        let xlen = (data[pos] as usize) | ((data[pos + 1] as usize) << 8);
        pos += 2 + xlen;
    }
    if flg & GZIP_FNAME != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        pos += 1; // skip the NUL terminator
    }
    if flg & GZIP_FCOMMENT != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        pos += 1;
    }
    if flg & GZIP_FHCRC != 0 {
        pos += 2;
    }
    if pos + 8 > data.len() {
        return Err(DecompressError::TooShort);
    }
    // The DEFLATE stream is everything between `pos` and the trailing
    // 8-byte CRC32 + ISIZE.
    let deflate_end = data.len() - 8;
    let out = inflate(&data[pos..deflate_end])?;
    // Verify CRC32 (last 8 bytes: CRC32 little-endian, ISIZE little-endian).
    let expected_crc = u32::from_le_bytes([
        data[deflate_end],
        data[deflate_end + 1],
        data[deflate_end + 2],
        data[deflate_end + 3],
    ]);
    let expected_size = u32::from_le_bytes([
        data[deflate_end + 4],
        data[deflate_end + 5],
        data[deflate_end + 6],
        data[deflate_end + 7],
    ]);
    let actual_crc = crc32(&out);
    if actual_crc != expected_crc {
        return Err(DecompressError::GzipChecksumMismatch);
    }
    if (out.len() as u32) != expected_size {
        return Err(DecompressError::GzipChecksumMismatch);
    }
    let _ = GZIP_FTEXT; // FTEXT is informational only; no action needed.
    Ok(out)
}

// ---------------------------------------------------------------------------
// ZIP (PKZIP)
// ---------------------------------------------------------------------------

/// ZIP local file header signature: `0x04034B50` (little-endian `PK\x03\x04`).
const ZIP_LOCAL_SIG: u32 = 0x04034B50;
/// ZIP compression method: stored (no compression).
const ZIP_METHOD_STORED: u16 = 0;
/// ZIP compression method: deflate.
const ZIP_METHOD_DEFLATE: u16 = 8;

/// Decompress a ZIP archive, returning the first `.nes` entry's payload
/// (or the first entry if none ends in `.nes`). Supports stored and
/// deflated entries.
pub fn decompress_zip(data: &[u8]) -> Result<Vec<u8>, DecompressError> {
    if data.len() < 30 {
        return Err(DecompressError::TooShort);
    }
    // Scan local file headers from the start of the archive.
    let mut pos = 0usize;
    let mut fallback: Option<(usize, usize)> = None; // (entry_pos, chosen_index)
    let mut entry_index = 0usize;
    while pos + 4 <= data.len() {
        let sig = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        if sig != ZIP_LOCAL_SIG {
            break;
        }
        if pos + 30 > data.len() {
            return Err(DecompressError::TooShort);
        }
        let method = u16::from_le_bytes([data[pos + 8], data[pos + 9]]);
        let compressed_size = u32::from_le_bytes([
            data[pos + 18],
            data[pos + 19],
            data[pos + 20],
            data[pos + 21],
        ]) as usize;
        let _uncompressed_size = u32::from_le_bytes([
            data[pos + 22],
            data[pos + 23],
            data[pos + 24],
            data[pos + 25],
        ]) as usize;
        let name_len = u16::from_le_bytes([data[pos + 26], data[pos + 27]]) as usize;
        let extra_len = u16::from_le_bytes([data[pos + 28], data[pos + 29]]) as usize;
        let data_start = pos + 30 + name_len + extra_len;
        if data_start + compressed_size > data.len() {
            return Err(DecompressError::TooShort);
        }
        let name_bytes = &data[pos + 30..pos + 30 + name_len];
        let name = String::from_utf8_lossy(name_bytes).to_string();
        let is_nes = name.to_ascii_lowercase().ends_with(".nes");

        if is_nes {
            return extract_zip_entry(data, data_start, compressed_size, method);
        }
        if fallback.is_none() {
            fallback = Some((data_start, compressed_size));
            let _ = entry_index; // tracked for completeness
        }
        entry_index += 1;
        pos = data_start + compressed_size;
    }
    if let Some((start, size)) = fallback {
        // No .nes entry — use the first file in the archive.
        // We need the method for the fallback entry; re-read it from the
        // local header at `start - 30 - name_len - extra_len`. Simpler:
        // re-scan from the top to find the first entry's method.
        if let Some(method) = first_zip_method(data) {
            return extract_zip_entry(data, start, size, method);
        }
    }
    Err(DecompressError::NoFileInZip)
}

/// Re-scan the first local file header to recover its compression method
/// (used by the fallback path when no `.nes` entry was found).
fn first_zip_method(data: &[u8]) -> Option<u16> {
    if data.len() < 30 {
        return None;
    }
    let sig = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if sig != ZIP_LOCAL_SIG {
        return None;
    }
    Some(u16::from_le_bytes([data[8], data[9]]))
}

/// Extract one ZIP entry given its data slice, compressed size, and
/// compression method.
fn extract_zip_entry(
    _data: &[u8],
    start: usize,
    size: usize,
    method: u16,
) -> Result<Vec<u8>, DecompressError> {
    let entry = &_data[start..start + size];
    match method {
        ZIP_METHOD_STORED => Ok(entry.to_vec()),
        ZIP_METHOD_DEFLATE => inflate(entry),
        _ => Err(DecompressError::UnsupportedZipMethod(method)),
    }
}

// ---------------------------------------------------------------------------
// CRC32 (used for Gzip verification)
// ---------------------------------------------------------------------------

/// Compute the CRC32 (IEEE 802.3 polynomial, reflected) of `data`. Used
/// to verify Gzip trailers and to build valid Gzip/ZIP trailers in
/// integration tests.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ---------------------------------------------------------------------------
// Format auto-detection
// ---------------------------------------------------------------------------

/// Detect the compression format of a byte slice by its magic bytes and
/// decompress accordingly. Returns the decompressed payload, or an error
/// if the format is unrecognized.
///
/// - `0x1F 0x8B` → Gzip
/// - `PK\x03\x04` → ZIP
/// - otherwise → `BadMagic` (caller should treat the input as raw)
pub fn decompress_auto(data: &[u8]) -> Result<Vec<u8>, DecompressError> {
    if data.len() >= 2 && data[0] == GZIP_MAGIC[0] && data[1] == GZIP_MAGIC[1] {
        decompress_gzip(data)
    } else if data.len() >= 4 && &data[..4] == b"PK\x03\x04" {
        decompress_zip(data)
    } else {
        Err(DecompressError::BadMagic)
    }
}

/// Returns `true` if `data` starts with a recognized compression magic
/// (Gzip or ZIP). Used by the cartridge loader to decide whether to
/// attempt decompression.
pub fn is_compressed(data: &[u8]) -> bool {
    if data.len() >= 2 && data[0] == GZIP_MAGIC[0] && data[1] == GZIP_MAGIC[1] {
        return true;
    }
    data.len() >= 4 && &data[..4] == b"PK\x03\x04"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal stored-block DEFLATE stream wrapping `payload`.
    /// BFINAL=1, BTYPE=00, then LEN/NLEN + data.
    fn deflate_stored(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        // BFINAL=1, BTYPE=00 → byte 0b00000001 (LSB-first: bit0=1, bits1-2=0).
        out.push(0x01);
        let len = payload.len() as u16;
        let nlen = !len;
        out.push(len as u8);
        out.push((len >> 8) as u8);
        out.push(nlen as u8);
        out.push((nlen >> 8) as u8);
        out.extend_from_slice(payload);
        out
    }

    /// Build a minimal Gzip file wrapping `deflate_stream` with the given
    /// original payload (for CRC/size computation).
    fn gzip_wrap(deflate_stream: &[u8], original: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&GZIP_MAGIC);
        out.push(GZIP_METHOD_DEFLATE); // CM
        out.push(0); // FLG
        out.extend_from_slice(&[0, 0, 0, 0]); // MTIME
        out.push(0); // XFL
        out.push(0xFF); // OS = unknown
        out.extend_from_slice(deflate_stream);
        let crc = crc32(original);
        out.extend_from_slice(&crc.to_le_bytes());
        let size = (original.len() as u32).to_le_bytes();
        out.extend_from_slice(&size);
        out
    }

    /// Build a minimal ZIP file containing one entry with the given name
    /// and (already-deflated or stored) payload.
    fn zip_wrap(name: &str, method: u16, payload: &[u8], original: &[u8]) -> Vec<u8> {
        let name_bytes = name.as_bytes();
        let crc = crc32(original);
        let mut out = Vec::new();
        out.extend_from_slice(&ZIP_LOCAL_SIG.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&[0, 0]); // mod time
        out.extend_from_slice(&[0, 0]); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes()); // compressed size
        out.extend_from_slice(&(original.len() as u32).to_le_bytes()); // uncompressed size
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn inflate_stored_block_round_trip() {
        let payload = b"Hello, DEFLATE!";
        let stream = deflate_stored(payload);
        let out = inflate(&stream).expect("inflate");
        assert_eq!(out, payload);
    }

    #[test]
    fn inflate_empty_stored_block() {
        let stream = deflate_stored(&[]);
        let out = inflate(&stream).expect("inflate");
        assert!(out.is_empty());
    }

    #[test]
    fn inflate_rejects_bad_block_type() {
        // BFINAL=1, BTYPE=11 → 0b00000111 = 0x07.
        let stream = [0x07u8];
        assert!(matches!(
            inflate(&stream),
            Err(DecompressError::InvalidBlockType)
        ));
    }

    #[test]
    fn inflate_rejects_stored_len_mismatch() {
        let mut stream = vec![0x01]; // BFINAL=1, BTYPE=00
        stream.extend_from_slice(&[0x05, 0x00]); // LEN=5
        stream.extend_from_slice(&[0x00, 0xFF]); // NLEN != ~LEN
        stream.extend_from_slice(&[1, 2, 3, 4, 5]);
        assert!(matches!(
            inflate(&stream),
            Err(DecompressError::StoredLenMismatch)
        ));
    }

    #[test]
    fn gzip_round_trip_stored() {
        let payload = b"NES ROM image data here";
        let stream = deflate_stored(payload);
        let gz = gzip_wrap(&stream, payload);
        let out = decompress_gzip(&gz).expect("decompress");
        assert_eq!(out, payload);
    }

    #[test]
    fn gzip_rejects_bad_magic() {
        let mut gz = gzip_wrap(&deflate_stored(b"hi"), b"hi");
        gz[0] = 0;
        assert!(matches!(
            decompress_gzip(&gz),
            Err(DecompressError::BadMagic)
        ));
    }

    #[test]
    fn gzip_rejects_crc_mismatch() {
        let payload = b"some data";
        let stream = deflate_stored(payload);
        let mut gz = gzip_wrap(&stream, payload);
        // Corrupt the CRC (last 8 bytes are CRC + ISIZE).
        let last = gz.len() - 8;
        gz[last] ^= 0xFF;
        assert!(matches!(
            decompress_gzip(&gz),
            Err(DecompressError::GzipChecksumMismatch)
        ));
    }

    #[test]
    fn gzip_handles_fname_field() {
        let payload = b"rom";
        let stream = deflate_stored(payload);
        // Build a gzip with FNAME flag set.
        let name = b"game.nes\0";
        let mut gz = Vec::new();
        gz.extend_from_slice(&GZIP_MAGIC);
        gz.push(GZIP_METHOD_DEFLATE);
        gz.push(GZIP_FNAME); // FLG
        gz.extend_from_slice(&[0, 0, 0, 0]); // MTIME
        gz.push(0);
        gz.push(0xFF);
        gz.extend_from_slice(name);
        gz.extend_from_slice(&stream);
        let crc = crc32(payload);
        gz.extend_from_slice(&crc.to_le_bytes());
        gz.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        let out = decompress_gzip(&gz).expect("decompress with FNAME");
        assert_eq!(out, payload);
    }

    #[test]
    fn zip_stored_round_trip() {
        let payload = b"NES ROM";
        let zip = zip_wrap("game.nes", ZIP_METHOD_STORED, payload, payload);
        let out = decompress_zip(&zip).expect("decompress");
        assert_eq!(out, payload);
    }

    #[test]
    fn zip_deflated_round_trip() {
        let payload = b"NES ROM image data with some repetition repetition repetition";
        let stream = deflate_stored(payload);
        let zip = zip_wrap("game.nes", ZIP_METHOD_DEFLATE, &stream, payload);
        let out = decompress_zip(&zip).expect("decompress");
        assert_eq!(out, payload);
    }

    #[test]
    fn zip_picks_nes_entry_over_other() {
        let readme = b"readme text";
        let rom = b"NES ROM";
        let mut zip = zip_wrap("readme.txt", ZIP_METHOD_STORED, readme, readme);
        zip.extend_from_slice(&zip_wrap("game.nes", ZIP_METHOD_STORED, rom, rom));
        let out = decompress_zip(&zip).expect("decompress");
        assert_eq!(out, rom);
    }

    #[test]
    fn zip_falls_back_to_first_entry_when_no_nes() {
        let payload = b"some file";
        let zip = zip_wrap("data.bin", ZIP_METHOD_STORED, payload, payload);
        let out = decompress_zip(&zip).expect("decompress");
        assert_eq!(out, payload);
    }

    #[test]
    fn zip_rejects_unsupported_method() {
        let zip = zip_wrap("game.nes", 1, &[0, 0], &[0, 0]);
        assert!(matches!(
            decompress_zip(&zip),
            Err(DecompressError::UnsupportedZipMethod(1))
        ));
    }

    #[test]
    fn decompress_auto_detects_gzip() {
        let payload = b"auto gzip";
        let gz = gzip_wrap(&deflate_stored(payload), payload);
        let out = decompress_auto(&gz).expect("auto");
        assert_eq!(out, payload);
    }

    #[test]
    fn decompress_auto_detects_zip() {
        let payload = b"auto zip";
        let zip = zip_wrap("game.nes", ZIP_METHOD_STORED, payload, payload);
        let out = decompress_auto(&zip).expect("auto");
        assert_eq!(out, payload);
    }

    #[test]
    fn decompress_auto_rejects_unknown() {
        let data = b"NOT A COMPRESSED FILE";
        assert!(matches!(
            decompress_auto(data),
            Err(DecompressError::BadMagic)
        ));
    }

    #[test]
    fn is_compressed_detects_both_formats() {
        let payload = b"x";
        let gz = gzip_wrap(&deflate_stored(payload), payload);
        let zip = zip_wrap("a.nes", ZIP_METHOD_STORED, payload, payload);
        assert!(is_compressed(&gz));
        assert!(is_compressed(&zip));
        assert!(!is_compressed(b"NES\x1A raw rom data"));
    }

    #[test]
    fn crc32_known_vectors() {
        // CRC32 of "123456789" is 0xCBF43926.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        // CRC32 of empty input is 0.
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn inflate_fixed_huffman_literal_only() {
        // Build a fixed-Huffman block containing only literal bytes for
        // "AB" and an end-of-block symbol (256). We construct the bit
        // stream manually.
        // 'A' = 0x41 → symbol 65, fixed code: 8 bits, code = 0x41 + 0x30 = 0x71?
        // Per RFC 1951 3.2.6: literal 0-143 → 8-bit codes 0x30 + sym.
        // 'A'(65) → 0x30+65 = 0x71 = 0b01110001 (8 bits, MSB-first).
        // 'B'(66) → 0x30+66 = 0x72 = 0b01110010.
        // EOB(256) → 7-bit code 0b0000000.
        // Pack LSB-first into bytes:
        // bits: 1,0,0,0,1,1,1,0 (A reversed), 0,1,0,0,1,1,1,0 (B reversed),
        // 0,0,0,0,0,0,0 (EOB), then BFINAL=1,BTYPE=01 at the very start.
        // BFINAL=1 (1 bit), BTYPE=01 (2 bits). `read_bits(2)` returns
        // value 0b01 = 1, which means stream bits are 1 (bit0), 0 (bit1).
        let mut bits: Vec<u8> = vec![1, 1, 0]; // BFINAL=1, BTYPE=01
                                               // 'A' code MSB-first: 0b01110001 → bits 0,1,1,1,0,0,0,1
        for b in [0u8, 1, 1, 1, 0, 0, 0, 1] {
            bits.push(b);
        }
        // 'B' code MSB-first: 0b01110010 → bits 0,1,1,1,0,0,1,0
        for b in [0u8, 1, 1, 1, 0, 0, 1, 0] {
            bits.push(b);
        }
        // EOB code MSB-first: 0b0000000 (7 bits)
        bits.extend(std::iter::repeat(0).take(7));
        // Pack bits LSB-first into bytes.
        let mut stream = Vec::new();
        for chunk in bits.chunks(8) {
            let mut byte: u8 = 0;
            for (&b, i) in chunk.iter().zip(0..) {
                byte |= b << i;
            }
            stream.push(byte);
        }
        let out = inflate(&stream).expect("inflate fixed");
        assert_eq!(out, b"AB");
    }

    /// Decompress a real gzip file produced by the system `gzip` tool.
    /// This exercises dynamic-Huffman blocks (the default for `gzip` on
    /// non-trivial input). The test is skipped if `gzip` is not installed.
    #[test]
    fn real_gzip_dynamic_huffman() {
        use std::process::Command;
        // Use a payload with repetition to encourage back-references.
        let mut payload = Vec::new();
        payload.extend_from_slice(b"NES\x1A");
        for _ in 0..200 {
            payload.extend_from_slice(b"ABABABABABABABAB");
        }
        let dir = std::env::temp_dir().join(format!("nes-emu-gzip-real-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let raw_path = dir.join("rom.nes");
        let gz_path = dir.join("rom.nes.gz");
        std::fs::write(&raw_path, &payload).unwrap();
        // Compress with the system gzip if available; skip otherwise.
        let gzip_available = Command::new("gzip")
            .arg("-kf")
            .arg(&raw_path)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !gzip_available {
            eprintln!("skipping real_gzip_dynamic_huffman: gzip not available");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let gz = std::fs::read(&gz_path).expect("read gz");
        let out = decompress_gzip(&gz).expect("decompress real gzip");
        assert_eq!(out, payload);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Decompress a real zip file produced by the system `zip` tool.
    /// Skipped if `zip` is not installed.
    #[test]
    fn real_zip_dynamic_huffman() {
        use std::process::Command;
        let mut payload = Vec::new();
        payload.extend_from_slice(b"NES\x1A");
        for _ in 0..200 {
            payload.extend_from_slice(b"XYZXYZXYZXYZ");
        }
        let dir = std::env::temp_dir().join(format!("nes-emu-zip-real-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rom_path = dir.join("game.nes");
        std::fs::write(&rom_path, &payload).unwrap();
        let zip_available = Command::new("zip")
            .arg("-q")
            .arg("-j")
            .arg(dir.join("archive.zip"))
            .arg(&rom_path)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !zip_available {
            eprintln!("skipping real_zip_dynamic_huffman: zip not available");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let zip = std::fs::read(dir.join("archive.zip")).expect("read zip");
        let out = decompress_zip(&zip).expect("decompress real zip");
        assert_eq!(out, payload);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
