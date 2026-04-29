//! TUZ v1 decompressor.
//!
//! Reads 2-byte LE header, then processes a bit stream of type bits,
//! variable-length codes, and literal bytes.
//!
//! Type bit 1 = literal byte, type bit 0 = dictionary match or control code.
//! Control codes (detected when decoded position == 0):
//!   - 1 = literal line (bulk literal run)
//!   - 3 = stream end

use super::bitstream::BitInputStream;

/// Header size for v1 format.
const HEADER_SIZE: usize = 2;

/// Minimum match length (same as encoder).
const MIN_MATCH_LEN: usize = 2;

/// Minimum literal-line length.
const MIN_LITERAL_LEN: usize = 15;

/// Large-position threshold for v1 format.
const LARGE_POS_THRESHOLD: usize = 0x2A7F; // 10879

/// Control: literal line.
const CTRL_LITERAL_LINE: usize = 1;

/// Control: stream end.
const CTRL_STREAM_END: usize = 3;

/// Decompress TUZ v1 compressed data.
///
/// Returns `Err` if the data is too short, contains invalid back-references,
/// or has an unknown control code.
pub fn tuz_decompress(compressed: &[u8]) -> Result<Vec<u8>, String> {
    if compressed.len() < HEADER_SIZE {
        return Err("compressed data too short for header".into());
    }

    // Skip 2-byte LE header (dict_size — informational, not needed for decoding)
    let mut reader = BitInputStream::new(&compressed[HEADER_SIZE..]);
    let mut output: Vec<u8> = Vec::new();

    // Safety: truncated input (no CTRL_STREAM_END) terminates because
    // reads past the end return 0, producing control code 0 → Err.
    loop {
        let type_bit = reader.read_bit();

        if type_bit == 1 {
            // Literal byte
            output.push(reader.read_byte());
        } else {
            // Dictionary match or control
            let saved_len = unpack_var_len(&mut reader, 1);

            // Read position
            let position = read_position(&mut reader);

            if position == 0 {
                // Control code
                match saved_len {
                    CTRL_LITERAL_LINE => {
                        let count = unpack_var_len(&mut reader, 2) + MIN_LITERAL_LEN;
                        for _ in 0..count {
                            output.push(reader.read_byte());
                        }
                    }
                    CTRL_STREAM_END => {
                        break;
                    }
                    other => {
                        return Err(format!("unknown control code: {other}"));
                    }
                }
            } else {
                // Dictionary match
                let mut match_len = saved_len;
                if position > LARGE_POS_THRESHOLD {
                    match_len += 1;
                }
                let match_len = match_len + MIN_MATCH_LEN;

                if position > output.len() {
                    return Err(format!(
                        "invalid back-reference: distance={position}, output_len={}",
                        output.len()
                    ));
                }

                let src_start = output.len() - position;
                for i in 0..match_len {
                    let b = output[src_start + i];
                    output.push(b);
                }
            }
        }
    }

    Ok(output)
}

/// Read variable-length integer using groups of `pack_bit` data bits + 1
/// continuation bit.
///
/// Groups are written MSB-first by the encoder, so decoding accumulates:
///   v = (v << pack_bit) + data_bits
///   if continuation bit set: v += 1, continue
fn unpack_var_len(reader: &mut BitInputStream, pack_bit: u32) -> usize {
    let mut v: usize = 0;
    let data_mask: usize = (1 << pack_bit) - 1;
    loop {
        // Read pack_bit data bits (LSB first from the bit stream)
        let mut data_bits: usize = 0;
        for i in 0..pack_bit {
            data_bits |= (reader.read_bit() as usize) << i;
        }
        // Read 1 continuation bit
        let cont = reader.read_bit();

        v = (v << pack_bit) | (data_bits & data_mask);

        if cont == 0 {
            return v;
        }
        v += 1;
    }
}

/// Read dictionary position from the bit stream.
///
/// Format:
///   - 1 indicator bit: 0 = small (< 128), 1 = large (>= 128)
///   - If small: 7 data bits → position value
///   - If large: 7 low bits + variable-length extension via 2-bit groups,
///     then position = (ext << 7 | low7) + 128
fn read_position(reader: &mut BitInputStream) -> usize {
    let indicator = reader.read_bit();
    // Read 7 low bits
    let mut low7: usize = 0;
    for i in 0..7 {
        low7 |= (reader.read_bit() as usize) << i;
    }

    if indicator == 0 {
        // Small position: value is just the 7 bits
        low7
    } else {
        // Large position: extend with variable-length 2-bit groups
        let ext = unpack_var_len(reader, 2);
        ((ext << 7) | low7) + 128
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompress_rejects_too_short() {
        assert!(tuz_decompress(&[]).is_err());
        assert!(tuz_decompress(&[0]).is_err());
    }

    // NOTE: The old bit-packed decoder is no longer compatible with the new
    // interleaved TUZ encoder (which matches the reference sisong/tinyuz format).
    // Round-trip validation is done in encoder::tests using a reference decompressor.
    // This decoder module will be updated or removed in a future commit.
}
