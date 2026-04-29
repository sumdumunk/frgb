//! TUZ v1 encoder — 2-byte header bit-stream format that Lian Li fan firmware decodes.
//!
//! The main encoder (encoder.rs) matches yuz.dll's 4-byte header interleaved format.
//! Fan firmware does NOT decode that format. This v1 encoder produces the original
//! format from the working Python implementation (commit 973f9ae).
//!
//! Key differences from v2:
//! - 2-byte dict_size header (not 4)
//! - Pure bit-stream encoding (no interleaved type bytes)
//! - Different variable-length integer encoding (2-bit and 3-bit groups)
//! - No back-reference optimization
//! - BIG_POS_FOR_LEN = 10879 (not 2687)

use super::matcher::{MatchFinder, TUZ_MIN_MATCH_LEN};

const MIN_LITERAL_LEN: usize = 15;
const BIG_POS_FOR_LEN: usize = 0x2A7F; // 10879

const CODE_DICT: u8 = 0;
const CODE_DATA: u8 = 1;

const CTRL_LITERAL_LINE: usize = 1;
const CTRL_STREAM_END: usize = 3;

// ---------------------------------------------------------------------------
// Bit output stream — LSB-first packing
// ---------------------------------------------------------------------------

struct BitOut {
    data: Vec<u8>,
    buffer: u8,
    bit_count: u8,
}

impl BitOut {
    fn new() -> Self {
        Self { data: Vec::new(), buffer: 0, bit_count: 0 }
    }

    fn write_bit(&mut self, bit: u8) {
        if bit != 0 {
            self.buffer |= 1 << self.bit_count;
        }
        self.bit_count += 1;
        if self.bit_count == 8 {
            self.data.push(self.buffer);
            self.buffer = 0;
            self.bit_count = 0;
        }
    }

    fn write_bits(&mut self, value: usize, num_bits: u8) {
        for i in 0..num_bits {
            self.write_bit(((value >> i) & 1) as u8);
        }
    }

    fn write_byte(&mut self, value: u8) {
        self.write_bits(value as usize, 8);
    }

    fn write_bytes(&mut self, data: &[u8]) {
        for &b in data {
            self.write_byte(b);
        }
    }

    fn flush(mut self) -> Vec<u8> {
        if self.bit_count > 0 {
            self.data.push(self.buffer);
        }
        self.data
    }
}

// ---------------------------------------------------------------------------
// Variable-length integer encoding
// ---------------------------------------------------------------------------

/// Write variable-length integer using 2-bit groups (1 data bit + 1 continuation bit).
fn write_len_2bit(out: &mut BitOut, value: usize) {
    let mut groups = Vec::new();
    let mut v = value;
    loop {
        groups.push(v & 1);
        v >>= 1;
        if v == 0 {
            break;
        }
        v -= 1;
    }
    for i in (0..groups.len()).rev() {
        out.write_bit(groups[i] as u8);
        out.write_bit(if i > 0 { 1 } else { 0 });
    }
}

/// Write variable-length integer using 3-bit groups (2 data bits + 1 continuation bit).
fn write_len_3bit(out: &mut BitOut, value: usize) {
    let mut groups = Vec::new();
    let mut v = value;
    loop {
        groups.push(v & 3);
        v >>= 2;
        if v == 0 {
            break;
        }
        v -= 1;
    }
    for i in (0..groups.len()).rev() {
        out.write_bits(groups[i], 2);
        out.write_bit(if i > 0 { 1 } else { 0 });
    }
}

// ---------------------------------------------------------------------------
// Token types for greedy parse
// ---------------------------------------------------------------------------

enum Token {
    Literal(u8),
    Match { distance: usize, length: usize },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compress data using the v1 TUZ format (2-byte header, bit-stream).
///
/// This is the format that Lian Li fan firmware can decode.
pub fn tuz_compress_v1(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.is_empty() {
        let mut out = BitOut::new();
        out.write_byte(0);
        out.write_byte(0);
        out.write_bit(CODE_DICT);
        write_len_2bit(&mut out, CTRL_STREAM_END);
        return Ok(out.flush());
    }

    let mut match_finder = MatchFinder::new(data, Some(15)); // dict_bits=15

    // Greedy parse
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let m = match_finder.find_best_match(pos);
        if m.is_valid() {
            tokens.push(Token::Match { distance: m.distance, length: m.length });
            pos += m.length;
        } else {
            tokens.push(Token::Literal(data[pos]));
            pos += 1;
        }
    }

    // Coalesce consecutive literals into literal lines
    let encoded = coalesce_literals(data, &tokens);

    // Encode
    let mut out = BitOut::new();

    // 2-byte dict_size header
    let dict_size: u16 = 1 << 15; // 32KB
    out.write_byte((dict_size & 0xFF) as u8);
    out.write_byte(((dict_size >> 8) & 0xFF) as u8);

    for item in &encoded {
        match item {
            EncodedItem::LiteralLine(bytes) => {
                out.write_bit(CODE_DICT);
                write_len_2bit(&mut out, CTRL_LITERAL_LINE);
                write_len_3bit(&mut out, bytes.len() - MIN_LITERAL_LEN);
                out.write_bytes(bytes);
            }
            EncodedItem::Literal(b) => {
                out.write_bit(CODE_DATA);
                out.write_byte(*b);
            }
            EncodedItem::Match { distance, length } => {
                out.write_bit(CODE_DICT);
                let mut adj_length = length - TUZ_MIN_MATCH_LEN;
                if *distance > BIG_POS_FOR_LEN && adj_length > 0 {
                    adj_length -= 1;
                }
                write_len_2bit(&mut out, adj_length);
                // Distance encoding
                let pos_val = distance - 1;
                if pos_val < 128 {
                    out.write_bit(0);
                    out.write_bits(pos_val, 7);
                } else {
                    out.write_bit(1);
                    write_len_3bit(&mut out, pos_val - 128);
                }
            }
        }
    }

    // Stream end
    out.write_bit(CODE_DICT);
    write_len_2bit(&mut out, CTRL_STREAM_END);

    Ok(out.flush())
}

// ---------------------------------------------------------------------------
// Literal coalescing
// ---------------------------------------------------------------------------

enum EncodedItem {
    LiteralLine(Vec<u8>),
    Literal(u8),
    Match { distance: usize, length: usize },
}

fn coalesce_literals(data: &[u8], tokens: &[Token]) -> Vec<EncodedItem> {
    let mut result = Vec::new();
    let mut literal_start: Option<usize> = None;
    let mut literal_count = 0usize;
    let mut data_pos = 0usize;

    for token in tokens {
        match token {
            Token::Literal(_) => {
                if literal_start.is_none() {
                    literal_start = Some(data_pos);
                }
                literal_count += 1;
                data_pos += 1;
            }
            Token::Match { distance, length } => {
                if literal_count > 0 {
                    flush_literals(data, literal_start.unwrap(), literal_count, &mut result);
                    literal_start = None;
                    literal_count = 0;
                }
                result.push(EncodedItem::Match { distance: *distance, length: *length });
                data_pos += length;
            }
        }
    }

    if literal_count > 0 {
        flush_literals(data, literal_start.unwrap(), literal_count, &mut result);
    }

    result
}

fn flush_literals(data: &[u8], start: usize, count: usize, result: &mut Vec<EncodedItem>) {
    if count >= MIN_LITERAL_LEN {
        result.push(EncodedItem::LiteralLine(data[start..start + count].to_vec()));
    } else {
        for i in 0..count {
            result.push(EncodedItem::Literal(data[start + i]));
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_v1_empty() {
        let compressed = tuz_compress_v1(&[]).unwrap();
        assert!(compressed.len() > 2); // at least header + stream end
    }

    #[test]
    fn compress_v1_static_red_24_leds_30_frames() {
        // Same test data as Python: 24 LEDs × 30 frames, all red
        let mut data = Vec::with_capacity(2160);
        for _ in 0..30 {
            for _ in 0..24 {
                data.extend_from_slice(&[254, 0, 0]);
            }
        }
        let compressed = tuz_compress_v1(&data).unwrap();
        // Compare with Python v1 output
        eprintln!("v1 compressed: {} bytes: {:02x?}", compressed.len(), &compressed);
    }
}
