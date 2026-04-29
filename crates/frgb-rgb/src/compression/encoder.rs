//! TUZ encoder — matches the reference sisong/tinyuz format exactly.
//!
//! The TUZ format interleaves bit-packed type/length data with byte-aligned
//! literal and position data in a single byte stream. Type bits are packed
//! into reserved "type bytes" (LSB-first, 8 bits each). Literals and
//! positions are pushed as raw bytes between type bytes.
//!
//! Reference: https://github.com/sisong/tinyuz

use super::matcher::{MatchFinder, TUZ_MIN_MATCH_LEN};

/// Minimum consecutive literals to trigger literal-line encoding.
const MIN_LITERAL_LEN: usize = 15;

/// Large-position threshold (reference: tuz_kBigPosForLen).
/// Positions above this get +1 length adjustment.
const BIG_POS_FOR_LEN: usize = (1 << 11) + (1 << 9) + (1 << 7) - 1; // 2687

/// Number of bytes for the dict_size header.
/// Must match the firmware's tuz_kDictSizeSavedBytes compile setting.
/// Default tinyuz builds with tuz_kMaxOfDictSize=(1<<30) → 4 bytes.
const DICT_SIZE_SAVED_BYTES: usize = 4;

/// Dictionary bits for match finding window.
/// USB capture shows L-Connect max distance ~4032. Using 12 bits (4096)
/// to stay within firmware decompressor limits. DICT_BITS=15 (32KB) caused
/// firmware crashes with match distances the decompressor couldn't handle.
const DICT_BITS: u32 = 12;

/// Control code: literal line.
const CTRL_LITERAL_LINE: usize = 1;

/// Control code: stream end.
const CTRL_STREAM_END: usize = 3;

/// TUZ output builder — interleaves type bits with raw bytes.
struct TuzCode {
    code: Vec<u8>,
    types_index: usize,
    type_count: usize,
    dict_pos_back: usize,
    is_have_data_back: bool,
    dict_size_max: usize,
}

impl TuzCode {
    fn new() -> Self {
        Self {
            code: Vec::new(),
            types_index: 0,
            type_count: 0,
            dict_pos_back: 1,
            is_have_data_back: false,
            dict_size_max: 1,
        }
    }

    /// Write a single type bit into the packed type byte.
    fn out_type(&mut self, bit: usize) {
        if self.type_count == 0 {
            self.types_index = self.code.len();
            self.code.push(0);
        }
        self.code[self.types_index] |= ((bit & 1) as u8) << self.type_count;
        self.type_count += 1;
        if self.type_count == 8 {
            self.type_count = 0;
        }
    }

    /// Write dict_size header (little-endian, DICT_SIZE_SAVED_BYTES bytes).
    fn out_dict_size(&mut self, dict_size: usize) {
        let mut v = dict_size;
        for _ in 0..DICT_SIZE_SAVED_BYTES {
            self.code.push((v & 0xFF) as u8);
            v >>= 8;
        }
    }

    /// Write a variable-length integer using groups of `pack_bit` data bits + 1 continuation bit.
    /// Reference: TTuzCode::outLen
    fn out_len(&mut self, value: usize, pack_bit: usize) {
        // Calculate number of groups and the "dec" offset
        let (count, dec) = get_out_count(value, pack_bit);
        let v = value - dec;

        // Output groups MSB-first
        let mut c = count;
        while c > 0 {
            c -= 1;
            for i in 0..pack_bit {
                self.out_type((v >> (c * pack_bit + i)) & 1);
            }
            self.out_type(if c > 0 { 1 } else { 0 });
        }
    }

    /// Write a dict position as a raw byte (+ optional var_len extension).
    /// Reference: TTuzCode::outDictPos
    fn out_dict_pos(&mut self, pos: usize) {
        let is_large = pos >= 128;
        let pos_adjusted = if is_large { pos - 128 } else { pos };
        self.code
            .push(((pos_adjusted & 0x7F) | if is_large { 0x80 } else { 0 }) as u8);
        if is_large {
            self.out_len(pos_adjusted >> 7, 2); // kDictPosLenPackBit = 2
        }
    }

    /// Write literal data bytes.
    /// Reference: TTuzCode::outData
    fn out_data(&mut self, data: &[u8]) {
        if data.len() >= MIN_LITERAL_LEN {
            // Literal line encoding
            self.out_ctrl(CTRL_LITERAL_LINE);
            self.out_len(data.len() - MIN_LITERAL_LEN, 2); // kDictPosLenPackBit = 2
            self.code.extend_from_slice(data);
        } else {
            // Individual literals
            for &b in data {
                self.out_type(1); // codeType_data
                self.code.push(b);
            }
        }
        self.is_have_data_back = true;
    }

    /// Write a dictionary match.
    /// Reference: TTuzCode::outDict
    fn out_dict(&mut self, match_len: usize, dict_pos: usize) {
        self.out_type(0); // codeType_dict

        let saved_dict_pos = dict_pos + 1; // 0 reserved for ctrl
        if saved_dict_pos > self.dict_size_max {
            self.dict_size_max = saved_dict_pos;
        }
        let is_same_pos = self.dict_pos_back == saved_dict_pos;
        let is_saved_same_pos = is_same_pos && self.is_have_data_back;

        let mut len = match_len - TUZ_MIN_MATCH_LEN;
        if !is_saved_same_pos && saved_dict_pos > BIG_POS_FOR_LEN {
            len -= 1; // large position length adjustment
        }

        self.out_len(len, 1); // kDictLenPackBit = 1

        if self.is_have_data_back {
            self.out_type(if is_saved_same_pos { 1 } else { 0 });
        }
        if !is_saved_same_pos {
            self.out_dict_pos(saved_dict_pos);
        }

        self.is_have_data_back = false;
        self.dict_pos_back = saved_dict_pos;
    }

    /// Write a control token (literal line or stream end).
    /// Reference: TTuzCode::outCtrl
    fn out_ctrl(&mut self, ctrl: usize) {
        self.out_type(0); // codeType_dict
        self.out_len(ctrl, 1); // kDictLenPackBit = 1
        if self.is_have_data_back {
            self.out_type(0); // back-ref bit = 0 for ctrl
        }
        self.out_dict_pos(0); // position 0 = ctrl marker
    }

    /// Write stream end and reset type state.
    /// Reference: TTuzCode::outCtrl_streamEnd + outCtrl_typesEnd
    fn out_stream_end(&mut self) {
        self.out_ctrl(CTRL_STREAM_END);
        // Reset type state (flush partial type byte)
        self.type_count = 0;
        self.dict_pos_back = 1;
        self.is_have_data_back = false;
    }

    fn finish(self) -> Vec<u8> {
        self.code
    }
}

/// Calculate number of var_len groups and the cumulative offset.
/// Reference: _getOutCount
fn get_out_count(value: usize, pack_bit: usize) -> (usize, usize) {
    let mut count = 1usize;
    let mut v = value;
    loop {
        let m = 1usize << (count * pack_bit);
        if v < m {
            break;
        }
        v -= m;
        count += 1;
    }
    (count, value - v)
}

/// Compress data using TUZ format matching the reference sisong/tinyuz.
pub fn tuz_compress(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = TuzCode::new();

    // Header: dict_size placeholder (updated after encoding with actual max distance).
    // The firmware's streaming TUZ decoder uses this to size its dictionary buffer.
    let dict_size = if data.is_empty() { 1 } else { 1usize << DICT_BITS };
    out.out_dict_size(dict_size);

    if data.is_empty() {
        out.out_stream_end();
        return Ok(out.finish());
    }

    // Greedy parse: find matches
    let mut match_finder = MatchFinder::new(data, Some(DICT_BITS));
    let mut tokens: Vec<Token> = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        let m = match_finder.find_best_match(pos);
        // saved_dict_pos = distance (our distances are 1-indexed = saved_dict_pos)
        // Skip 2-byte matches at large distances: decoder adds +1 to length for
        // large positions, so minimum encodable match is 3 bytes there.
        let at_big_pos = m.distance > BIG_POS_FOR_LEN;
        if m.is_valid() && !(m.length == TUZ_MIN_MATCH_LEN && at_big_pos) {
            tokens.push(Token::Match {
                distance: m.distance, // 1-indexed backward distance
                length: m.length,
            });
            for skip in 1..m.length {
                if pos + skip < data.len() {
                    match_finder.find_best_match(pos + skip);
                }
            }
            pos += m.length;
        } else {
            tokens.push(Token::Literal);
            pos += 1;
        }
    }

    // Emit tokens, coalescing consecutive literals
    let mut data_pos = 0;
    let mut literal_start: Option<usize> = None;
    let mut literal_count = 0;

    for token in &tokens {
        match token {
            Token::Literal => {
                if literal_start.is_none() {
                    literal_start = Some(data_pos);
                }
                literal_count += 1;
                data_pos += 1;
            }
            Token::Match { distance, length } => {
                if literal_count > 0 {
                    out.out_data(&data[literal_start.unwrap()..literal_start.unwrap() + literal_count]);
                    literal_start = None;
                    literal_count = 0;
                }
                // dict_pos is 0-indexed: distance=1 means previous byte = dict_pos 0
                out.out_dict(*length, *distance - 1);
                data_pos += length;
            }
        }
    }

    if literal_count > 0 {
        out.out_data(&data[literal_start.unwrap()..literal_start.unwrap() + literal_count]);
    }

    out.out_stream_end();

    // Update header with actual max distance used (matching protocol behavior).
    // The firmware's streaming TUZ decoder allocates dict_size bytes from the header.
    let actual = out.dict_size_max;
    let mut result = out.finish();
    for (i, byte) in result.iter_mut().take(DICT_SIZE_SAVED_BYTES).enumerate() {
        *byte = ((actual >> (i * 8)) & 0xFF) as u8;
    }
    Ok(result)
}

#[derive(Debug)]
enum Token {
    Literal,
    Match { distance: usize, length: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decompress using the reference algorithm (Rust port of tuz_decompress_mem).
    fn reference_decompress(code: &[u8], max_output: usize) -> Result<Vec<u8>, String> {
        if code.len() < DICT_SIZE_SAVED_BYTES {
            return Err("too short".into());
        }

        let mut pos = DICT_SIZE_SAVED_BYTES; // skip header
        let mut out = Vec::with_capacity(max_output);
        let mut types: u8 = 0;
        let mut type_count: u8 = 0;
        let mut dict_pos_back: usize = 1;
        let mut is_have_data_back = false;

        // Read bits from the code stream (LSB-first within bytes)
        macro_rules! read_lowbits {
            ($n:expr) => {{
                let bit_count: u8 = $n;
                let count = type_count;
                let result = types;
                if count >= bit_count {
                    type_count = count - bit_count;
                    types = result >> bit_count;
                    result
                } else {
                    if pos >= code.len() {
                        return Err("read past end".into());
                    }
                    let v = code[pos];
                    pos += 1;
                    let bc = bit_count - count;
                    type_count = 8 - bc;
                    types = v >> bc;
                    result | (v << count)
                }
            }};
        }

        macro_rules! unpack_len {
            ($read_bit:expr) => {{
                let rb: u8 = $read_bit;
                let mask = (1u8 << (rb - 1)) - 1;
                let cont_mask = 1u8 << (rb - 1);
                let mut v: usize = 0;
                loop {
                    let lowbit = read_lowbits!(rb);
                    v = (v << (rb - 1)) + (lowbit & mask) as usize;
                    if (lowbit & cont_mask) == 0 {
                        break;
                    }
                    v += 1;
                }
                v
            }};
        }

        loop {
            let type_bit = read_lowbits!(1) & 1;
            if type_bit == 0 {
                // dict/ctrl
                let saved_len = unpack_len!(2);
                let saved_dict_pos;
                let mut big_pos_adj = false;
                if is_have_data_back && (read_lowbits!(1) & 1) != 0 {
                    saved_dict_pos = dict_pos_back;
                } else {
                    if pos >= code.len() {
                        return Err("read past end for pos".into());
                    }
                    let pb = code[pos];
                    pos += 1;
                    if pb < 128 {
                        saved_dict_pos = pb as usize;
                    } else {
                        let ext = unpack_len!(3);
                        saved_dict_pos = ((pb as usize & 0x7F) | (ext << 7)) + 128;
                    }
                    if saved_dict_pos > BIG_POS_FOR_LEN {
                        big_pos_adj = true;
                    }
                }
                is_have_data_back = false;

                if saved_dict_pos != 0 {
                    let mut dict_len = saved_len;
                    if big_pos_adj {
                        dict_len += 1;
                    }
                    dict_len += TUZ_MIN_MATCH_LEN;
                    dict_pos_back = saved_dict_pos;
                    if saved_dict_pos > out.len() {
                        return Err(format!("dict pos {} > output len {}", saved_dict_pos, out.len()));
                    }
                    for _ in 0..dict_len {
                        let src_idx = out.len() - saved_dict_pos;
                        let b = out[src_idx];
                        out.push(b);
                    }
                } else {
                    // ctrl
                    if saved_len == CTRL_LITERAL_LINE {
                        let lit_len = unpack_len!(3) + MIN_LITERAL_LEN;
                        is_have_data_back = true;
                        if pos + lit_len > code.len() {
                            return Err("literal line overrun".into());
                        }
                        out.extend_from_slice(&code[pos..pos + lit_len]);
                        pos += lit_len;
                    } else {
                        dict_pos_back = 1;
                        type_count = 0;
                        if saved_len == CTRL_STREAM_END {
                            return Ok(out);
                        } else if saved_len == 2 {
                            // clip end — continue
                        } else {
                            return Err(format!("unknown ctrl type {}", saved_len));
                        }
                    }
                }
            } else {
                // literal
                is_have_data_back = true;
                if pos >= code.len() {
                    return Err("read past end for literal".into());
                }
                out.push(code[pos]);
                pos += 1;
            }
        }
    }

    #[test]
    fn compress_empty() {
        let compressed = tuz_compress(&[]).unwrap();
        let decompressed = reference_decompress(&compressed, 0).unwrap();
        assert_eq!(decompressed, vec![]);
    }

    #[test]
    fn compress_short_literal() {
        let data = vec![1, 2, 3, 4, 5];
        let compressed = tuz_compress(&data).unwrap();
        let decompressed = reference_decompress(&compressed, data.len()).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn compress_repeated_pattern() {
        let data: Vec<u8> = (0..100).map(|i| (i % 10) as u8).collect();
        let compressed = tuz_compress(&data).unwrap();
        let decompressed = reference_decompress(&compressed, data.len()).unwrap();
        assert_eq!(decompressed, data);
        assert!(compressed.len() < data.len());
    }

    #[test]
    fn compress_all_zeros() {
        let data = vec![0u8; 256];
        let compressed = tuz_compress(&data).unwrap();
        let decompressed = reference_decompress(&compressed, data.len()).unwrap();
        assert_eq!(decompressed, data);
        assert!(compressed.len() < data.len());
    }

    #[test]
    fn compress_rgb_like_data() {
        let mut data = Vec::with_capacity(2160);
        for _ in 0..30 {
            for _ in 0..24 {
                data.extend_from_slice(&[254, 0, 0]);
            }
        }
        let compressed = tuz_compress(&data).unwrap();
        let decompressed = reference_decompress(&compressed, data.len()).unwrap();
        assert_eq!(decompressed, data);
        assert!(compressed.len() < data.len() / 2);
    }

    #[test]
    fn compress_random_ish_data() {
        let data: Vec<u8> = (0..200).map(|i| ((i * 37 + 13) % 256) as u8).collect();
        let compressed = tuz_compress(&data).unwrap();
        let decompressed = reference_decompress(&compressed, data.len()).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn compress_large_data_roundtrip() {
        let data: Vec<u8> = (0..10000).map(|i| (i % 251) as u8).collect();
        let compressed = tuz_compress(&data).unwrap();
        let decompressed = reference_decompress(&compressed, data.len()).unwrap();
        assert_eq!(decompressed, data);
        assert!(compressed.len() < data.len());
    }

    #[test]
    fn compress_literal_line_trigger() {
        let data: Vec<u8> = (0u8..20).collect();
        let compressed = tuz_compress(&data).unwrap();
        let decompressed = reference_decompress(&compressed, data.len()).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn compress_large_data_with_short_matches_at_large_offsets() {
        let mut data = Vec::with_capacity(15000);
        for i in 0..15000u32 {
            data.push(((i * 17 + i / 256) % 256) as u8);
        }
        data[10] = 0xDE;
        data[11] = 0xAD;
        data[11500] = 0xDE;
        data[11501] = 0xAD;
        let compressed = tuz_compress(&data).unwrap();
        let decompressed = reference_decompress(&compressed, data.len()).unwrap();
        assert_eq!(decompressed.len(), data.len());
        assert_eq!(decompressed, data);
    }

    #[test]
    fn compress_sl_infinity_sized_data() {
        let mut data = Vec::with_capacity(29580);
        for frame in 0..170u32 {
            for led in 0..58u32 {
                let r = ((frame * 3 + led * 7) % 254) as u8;
                let g = ((frame * 5 + led * 11) % 254) as u8;
                let b = ((frame * 7 + led * 13) % 254) as u8;
                data.push(r);
                data.push(g);
                data.push(b);
            }
        }
        let compressed = tuz_compress(&data).unwrap();
        let decompressed = reference_decompress(&compressed, data.len()).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn compress_single_byte() {
        let data = vec![0xAB];
        let compressed = tuz_compress(&data).unwrap();
        let decompressed = reference_decompress(&compressed, data.len()).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn compress_static_color_dict_size_matches_max_distance() {
        // Static color: 3-byte repeat pattern → dict_size_max should be 3
        let mut data = Vec::with_capacity(10800);
        for _ in 0..3600 {
            data.extend_from_slice(&[254, 0, 0]);
        }
        let compressed = tuz_compress(&data).unwrap();
        // Header should be dict_size=3 (actual max distance)
        assert_eq!(&compressed[..4], &[3, 0, 0, 0]);
    }

    #[test]
    fn compress_breathing_dict_size() {
        // Generate breathing buffer: 170 frames, 24 LEDs, red fading
        let mut data = Vec::with_capacity(12240);
        for frame in 0..170usize {
            let ramp = if frame < 85 { frame } else { 169 - frame } as u8;
            let ramp_mul = ((ramp as u16 * 3) & 0xFF) as u8;
            let r = ((254u16 * ramp_mul as u16) >> 8) as u8;
            let r = ((r as u16 * 255u16) >> 8) as u8;
            for _ in 0..24 {
                data.push(r);
                data.push(0);
                data.push(0);
            }
        }
        let compressed = tuz_compress(&data).unwrap();
        let dict_size = u32::from_le_bytes([compressed[0], compressed[1], compressed[2], compressed[3]]);
        eprintln!(
            "breathing: compressed={} bytes, dict_size={}",
            compressed.len(),
            dict_size
        );
        eprintln!("capture dict_size=4032");
    }

    #[test]
    fn compress_matches_python_static_red() {
        // 24 LEDs × 30 frames, all red (254, 0, 0) — same as Python test
        let mut data = Vec::with_capacity(2160);
        for _ in 0..30 {
            for _ in 0..24 {
                data.extend_from_slice(&[254, 0, 0]);
            }
        }
        let compressed = tuz_compress(&data).unwrap();
        // Python produces: 03000000 a7 fe 00 00 ... (13 bytes total)
        let python_output: Vec<u8> = vec![
            0x03, 0x00, 0x00, 0x00, 0xa7, 0xfe, 0x00, 0x00, 0xfa, 0xbe, 0x61, 0x03, 0x00,
        ];
        eprintln!("rust:   {} bytes: {:02x?}", compressed.len(), &compressed);
        eprintln!("python: {} bytes: {:02x?}", python_output.len(), &python_output);
        assert_eq!(compressed.len(), python_output.len(), "compressed size mismatch");
        assert_eq!(compressed, python_output, "compressed bytes mismatch");
    }
}
