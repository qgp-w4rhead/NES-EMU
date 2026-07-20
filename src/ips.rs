//! IPS patch file parsing and application (M34).
//!
//! IPS (International Patching System) is the classic binary patch format
//! used by the NES emulation community for translations and ROM hacks. A
//! patch file is a sequence of records describing byte ranges to overwrite
//! in a base ROM image.
//!
//! # File layout
//!
//! ```text
//! "PATCH"                  5-byte header magic
//! record*                  one or more patch records
//! "EOF"                    3-byte trailer (0x45 0x4F 0x46)
//! ```
//!
//! Each record is:
//!
//! ```text
//! offset   3 bytes (big-endian, 24-bit address)
//! size     2 bytes (big-endian)
//! data     `size` bytes
//! ```
//!
//! A record with `size == 0` is an **RLE record**: the next 2 bytes are the
//! run length `n` (big-endian) and the byte after that is the fill value.
//! The patcher writes `n` copies of the fill value at `offset`.
//!
//! The trailer is the literal 3 bytes `"EOF"` (`0x454F46`). Some patch
//! files append extra metadata after the trailer (e.g. a 3-byte CRC); we
//! ignore any trailing bytes after the trailer.
//!
//! # Truncation / extension
//!
//! IPS patches may write past the end of the base ROM — the patched image
//! is grown to fit the highest written address. Patches never shrink the
//! ROM.
//!
//! See: https://www.nesdev.org/wiki/IPS
//! See: http://zerosoft.zophar.net/ips.php

use std::fmt;

/// IPS file header magic: the 5 ASCII bytes `"PATCH"`.
pub const IPS_MAGIC: &[u8; 5] = b"PATCH";
/// IPS trailer magic: the 3 ASCII bytes `"EOF"` (also a valid 24-bit
/// offset, `0x454F46`, which is why patches must never target that exact
/// address — the spec recommends padding the ROM first if needed).
pub const IPS_EOF: &[u8; 3] = b"EOF";
/// Maximum IPS record size (16-bit field, so 65535 is the largest non-RLE
/// payload). RLE runs can be up to 65535 bytes as well.
pub const IPS_MAX_RECORD_SIZE: usize = 0xFFFF;
/// Maximum ROM size growable by an IPS patch. IPS offsets are 24-bit, so
/// the maximum patched image is 16 MiB. We cap growth at 24-bit address
/// space to reject malformed patches that target absurd offsets.
pub const IPS_MAX_ROM_SIZE: usize = 0x100_0000;

/// A single IPS patch record — either a block of literal bytes or an RLE
/// fill run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpsRecord {
    /// Overwrite `data.len()` bytes starting at `offset` with `data`.
    Copy { offset: usize, data: Vec<u8> },
    /// Write `len` copies of `value` starting at `offset`.
    Rle {
        offset: usize,
        value: u8,
        len: usize,
    },
}

impl IpsRecord {
    /// Return the ROM offset where this record begins writing.
    pub fn offset(&self) -> usize {
        match self {
            IpsRecord::Copy { offset, .. } => *offset,
            IpsRecord::Rle { offset, .. } => *offset,
        }
    }
}

/// Errors that can occur while parsing or applying an IPS patch.
#[derive(Debug)]
pub enum IpsError {
    /// The patch buffer is too short to contain even the header + trailer.
    TooShort,
    /// The first 5 bytes are not `"PATCH"`.
    BadMagic,
    /// A record extended past the end of the patch buffer.
    TruncatedRecord,
    /// The patch did not end with an `"EOF"` trailer.
    MissingEof,
    /// A record targeted an offset beyond the 24-bit address space.
    OffsetOutOfRange,
    /// An RLE run length was zero (the spec forbids this — a zero `size`
    /// field is the RLE marker, so the run length must be ≥ 1).
    ZeroRleLength,
    /// The patched image would exceed the maximum supported ROM size.
    RomTooLarge,
}

impl fmt::Display for IpsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IpsError::TooShort => write!(f, "IPS patch too short for header + trailer"),
            IpsError::BadMagic => write!(f, "IPS magic 'PATCH' not found"),
            IpsError::TruncatedRecord => write!(f, "IPS record truncated"),
            IpsError::MissingEof => write!(f, "IPS trailer 'EOF' not found"),
            IpsError::OffsetOutOfRange => write!(f, "IPS record offset out of 24-bit range"),
            IpsError::ZeroRleLength => write!(f, "IPS RLE record has zero length"),
            IpsError::RomTooLarge => write!(f, "patched ROM exceeds maximum size"),
        }
    }
}

impl std::error::Error for IpsError {}

/// A parsed IPS patch — an ordered list of records.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IpsPatch {
    records: Vec<IpsRecord>,
}

impl IpsPatch {
    /// Parse an IPS patch from a byte slice. The patch must start with
    /// `"PATCH"` and end with `"EOF"` (trailing bytes after the trailer
    /// are ignored). Returns the ordered list of records.
    pub fn from_bytes(data: &[u8]) -> Result<Self, IpsError> {
        if data.len() < IPS_MAGIC.len() + IPS_EOF.len() {
            return Err(IpsError::TooShort);
        }
        if &data[..IPS_MAGIC.len()] != IPS_MAGIC {
            return Err(IpsError::BadMagic);
        }

        let mut records = Vec::new();
        let mut pos = IPS_MAGIC.len();
        let end = data.len();
        while pos + 3 <= end {
            // Read the 3-byte big-endian offset. If it equals "EOF"
            // (0x454F46) we've reached the trailer.
            let off = ((data[pos] as usize) << 16)
                | ((data[pos + 1] as usize) << 8)
                | (data[pos + 2] as usize);
            if &data[pos..pos + 3] == IPS_EOF {
                // Trailer found — ignore any trailing bytes.
                return Ok(IpsPatch { records });
            }
            pos += 3;

            // 2-byte big-endian size.
            if pos + 2 > end {
                return Err(IpsError::TruncatedRecord);
            }
            let size = ((data[pos] as usize) << 8) | (data[pos + 1] as usize);
            pos += 2;

            if size == 0 {
                // RLE record: 2-byte run length + 1-byte fill value.
                if pos + 3 > end {
                    return Err(IpsError::TruncatedRecord);
                }
                let len = ((data[pos] as usize) << 8) | (data[pos + 1] as usize);
                let value = data[pos + 2];
                pos += 3;
                if len == 0 {
                    return Err(IpsError::ZeroRleLength);
                }
                if off.saturating_add(len) > IPS_MAX_ROM_SIZE {
                    return Err(IpsError::OffsetOutOfRange);
                }
                records.push(IpsRecord::Rle {
                    offset: off,
                    value,
                    len,
                });
            } else {
                if pos + size > end {
                    return Err(IpsError::TruncatedRecord);
                }
                if off.saturating_add(size) > IPS_MAX_ROM_SIZE {
                    return Err(IpsError::OffsetOutOfRange);
                }
                let payload = data[pos..pos + size].to_vec();
                pos += size;
                records.push(IpsRecord::Copy {
                    offset: off,
                    data: payload,
                });
            }
        }
        // Ran out of bytes without finding the trailer.
        Err(IpsError::MissingEof)
    }

    /// Apply the patch to a base ROM image, growing it as needed. The
    /// patched image is returned as a new `Vec<u8>`; the input is left
    /// unmodified. Returns an error if the patched image would exceed
    /// [`IPS_MAX_ROM_SIZE`].
    pub fn apply(&self, base: &[u8]) -> Result<Vec<u8>, IpsError> {
        // Determine the final image size — the highest written address.
        let mut max_end = base.len();
        for rec in &self.records {
            let end = match rec {
                IpsRecord::Copy { offset, data } => offset.saturating_add(data.len()),
                IpsRecord::Rle { offset, len, .. } => offset.saturating_add(*len),
            };
            if end > max_end {
                max_end = end;
            }
        }
        if max_end > IPS_MAX_ROM_SIZE {
            return Err(IpsError::RomTooLarge);
        }

        let mut out = Vec::with_capacity(max_end);
        out.extend_from_slice(base);
        out.resize(max_end, 0u8);

        for rec in &self.records {
            match rec {
                IpsRecord::Copy { offset, data } => {
                    out[*offset..*offset + data.len()].copy_from_slice(data);
                }
                IpsRecord::Rle { offset, value, len } => {
                    for i in 0..*len {
                        out[*offset + i] = *value;
                    }
                }
            }
        }
        Ok(out)
    }

    /// Number of records in the patch.
    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    /// Borrow the patch records.
    pub fn records(&self) -> &[IpsRecord] {
        &self.records
    }

    /// Build a patch from an explicit record list. Useful for tests and
    /// for programmatically constructing patches.
    pub fn from_records(records: Vec<IpsRecord>) -> Self {
        IpsPatch { records }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a raw IPS byte stream from `(offset, bytes)` tuples.
    fn build_copy(records: &[(usize, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(IPS_MAGIC);
        for &(off, data) in records {
            out.push((off >> 16) as u8);
            out.push((off >> 8) as u8);
            out.push(off as u8);
            out.push((data.len() >> 8) as u8);
            out.push(data.len() as u8);
            out.extend_from_slice(data);
        }
        out.extend_from_slice(IPS_EOF);
        out
    }

    /// Build an RLE record bytes: offset, 0x0000, len_hi, len_lo, value.
    fn build_rle(off: usize, len: usize, value: u8) -> Vec<u8> {
        vec![
            (off >> 16) as u8,
            (off >> 8) as u8,
            off as u8,
            0,
            0,
            (len >> 8) as u8,
            len as u8,
            value,
        ]
    }

    #[test]
    fn parses_simple_copy_patch() {
        let bytes = build_copy(&[(0x10, &[0xAA, 0xBB, 0xCC])]);
        let patch = IpsPatch::from_bytes(&bytes).expect("parse");
        assert_eq!(patch.record_count(), 1);
        assert_eq!(
            patch.records()[0],
            IpsRecord::Copy {
                offset: 0x10,
                data: vec![0xAA, 0xBB, 0xCC],
            }
        );
    }

    #[test]
    fn parses_multiple_records() {
        let bytes = build_copy(&[(0, &[1, 2]), (0x100, &[3, 4, 5]), (0x200, &[6])]);
        let patch = IpsPatch::from_bytes(&bytes).expect("parse");
        assert_eq!(patch.record_count(), 3);
        assert_eq!(patch.records()[0].offset(), 0);
        assert_eq!(patch.records()[1].offset(), 0x100);
        assert_eq!(patch.records()[2].offset(), 0x200);
    }

    #[test]
    fn parses_rle_record() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(IPS_MAGIC);
        bytes.extend_from_slice(&build_rle(0x8000, 0x100, 0x42));
        bytes.extend_from_slice(IPS_EOF);
        let patch = IpsPatch::from_bytes(&bytes).expect("parse");
        assert_eq!(patch.record_count(), 1);
        assert_eq!(
            patch.records()[0],
            IpsRecord::Rle {
                offset: 0x8000,
                value: 0x42,
                len: 0x100,
            }
        );
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = build_copy(&[(0, &[1])]);
        bytes[0] = b'X';
        assert!(matches!(
            IpsPatch::from_bytes(&bytes),
            Err(IpsError::BadMagic)
        ));
    }

    #[test]
    fn rejects_too_short() {
        assert!(matches!(
            IpsPatch::from_bytes(b"PA"),
            Err(IpsError::TooShort)
        ));
    }

    #[test]
    fn rejects_missing_eof() {
        let mut bytes = build_copy(&[(0, &[1, 2])]);
        // Strip the trailer.
        bytes.truncate(bytes.len() - 3);
        assert!(matches!(
            IpsPatch::from_bytes(&bytes),
            Err(IpsError::MissingEof)
        ));
    }

    #[test]
    fn rejects_truncated_record() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(IPS_MAGIC);
        // Offset + size claiming 4 bytes of payload, but no payload.
        bytes.extend_from_slice(&[0x00, 0x10, 0x00, 0x00, 0x04]);
        assert!(matches!(
            IpsPatch::from_bytes(&bytes),
            Err(IpsError::TruncatedRecord)
        ));
    }

    #[test]
    fn rejects_zero_rle_length() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(IPS_MAGIC);
        // offset=0, size=0 (RLE marker), len=0, value=0.
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(IPS_EOF);
        assert!(matches!(
            IpsPatch::from_bytes(&bytes),
            Err(IpsError::ZeroRleLength)
        ));
    }

    #[test]
    fn ignores_trailing_bytes_after_eof() {
        let mut bytes = build_copy(&[(0, &[1])]);
        // Append some junk after the trailer (e.g. a CRC).
        bytes.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let patch = IpsPatch::from_bytes(&bytes).expect("parse");
        assert_eq!(patch.record_count(), 1);
    }

    #[test]
    fn apply_overwrites_base() {
        let bytes = build_copy(&[(2, &[0xAA, 0xBB])]);
        let patch = IpsPatch::from_bytes(&bytes).expect("parse");
        let base = vec![0, 0, 0, 0, 0];
        let out = patch.apply(&base).expect("apply");
        assert_eq!(out, vec![0, 0, 0xAA, 0xBB, 0]);
    }

    #[test]
    fn apply_grows_rom_when_writing_past_end() {
        let bytes = build_copy(&[(0x10, &[0xCC, 0xDD])]);
        let patch = IpsPatch::from_bytes(&bytes).expect("parse");
        let base = vec![0; 4];
        let out = patch.apply(&base).expect("apply");
        assert_eq!(out.len(), 0x12);
        assert_eq!(out[0x10], 0xCC);
        assert_eq!(out[0x11], 0xDD);
        // Bytes between base end and patch start are zero-filled.
        assert_eq!(out[4..0x10], vec![0u8; 0x10 - 4]);
    }

    #[test]
    fn apply_rle_fills_run() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(IPS_MAGIC);
        bytes.extend_from_slice(&build_rle(0, 5, 0x77));
        bytes.extend_from_slice(IPS_EOF);
        let patch = IpsPatch::from_bytes(&bytes).expect("parse");
        let base = vec![];
        let out = patch.apply(&base).expect("apply");
        assert_eq!(out, vec![0x77; 5]);
    }

    #[test]
    fn apply_preserves_unpatched_bytes() {
        let bytes = build_copy(&[(1, &[0xFF])]);
        let patch = IpsPatch::from_bytes(&bytes).expect("parse");
        let base = vec![0x11, 0x22, 0x33, 0x44];
        let out = patch.apply(&base).expect("apply");
        assert_eq!(out, vec![0x11, 0xFF, 0x33, 0x44]);
    }

    #[test]
    fn apply_does_not_mutate_base() {
        let bytes = build_copy(&[(0, &[0xAA])]);
        let patch = IpsPatch::from_bytes(&bytes).expect("parse");
        let base = vec![0; 4];
        let _ = patch.apply(&base).expect("apply");
        assert_eq!(base, vec![0; 4]);
    }

    #[test]
    fn empty_patch_applies_to_unchanged_base() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(IPS_MAGIC);
        bytes.extend_from_slice(IPS_EOF);
        let patch = IpsPatch::from_bytes(&bytes).expect("parse");
        assert_eq!(patch.record_count(), 0);
        let base = vec![1, 2, 3];
        let out = patch.apply(&base).expect("apply");
        assert_eq!(out, base);
    }

    #[test]
    fn round_trip_from_records_to_bytes_to_records() {
        let records = vec![
            IpsRecord::Copy {
                offset: 0x100,
                data: vec![0xDE, 0xAD],
            },
            IpsRecord::Rle {
                offset: 0x200,
                value: 0xBE,
                len: 4,
            },
        ];
        let patch = IpsPatch::from_records(records.clone());
        assert_eq!(patch.records(), records.as_slice());
    }
}
