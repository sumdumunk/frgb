/// Bit output stream with LSB-first packing.
///
/// Bits are written from LSB (bit 0) to MSB (bit 7) within each byte.
/// When a byte is full it is pushed to the output buffer and the next byte begins.
pub struct BitOutputStream {
    bytes: Vec<u8>,
    current: u8,
    bit_pos: u8, // 0-7: next bit position to write within `current`
}

impl BitOutputStream {
    pub fn new() -> Self {
        Self {
            bytes: Vec::new(),
            current: 0,
            bit_pos: 0,
        }
    }

    /// Write a single bit (only the LSB of `bit` is used).
    pub fn write_bit(&mut self, bit: u8) {
        if bit & 1 != 0 {
            self.current |= 1 << self.bit_pos;
        }
        self.bit_pos += 1;
        if self.bit_pos == 8 {
            self.bytes.push(self.current);
            self.current = 0;
            self.bit_pos = 0;
        }
    }

    /// Write a full byte.  If the stream is currently byte-aligned the byte is
    /// pushed directly; otherwise each bit is written individually.
    pub fn write_byte(&mut self, value: u8) {
        if self.bit_pos == 0 {
            self.bytes.push(value);
        } else {
            for i in 0..8 {
                self.write_bit((value >> i) & 1);
            }
        }
    }

    /// Write a slice of bytes.
    pub fn write_bytes(&mut self, data: &[u8]) {
        for &b in data {
            self.write_byte(b);
        }
    }

    /// Flush any partial byte (zero-padding the high bits) and return the
    /// complete byte buffer.
    pub fn finish(mut self) -> Vec<u8> {
        if self.bit_pos > 0 {
            self.bytes.push(self.current);
        }
        self.bytes
    }
}

impl Default for BitOutputStream {
    fn default() -> Self {
        Self::new()
    }
}

/// Bit input stream with LSB-first reading.
///
/// Bits are read from LSB (bit 0) to MSB (bit 7) within each byte.
/// Reading past the end of the data returns 0 without panicking.
pub struct BitInputStream<'a> {
    data: &'a [u8],
    byte_pos: usize, // index of the byte currently being read
    bit_pos: u8,     // 0-7: next bit position to read within data[byte_pos]
}

impl<'a> BitInputStream<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    /// Read a single bit.  Returns 0 when past end of data.
    pub fn read_bit(&mut self) -> u8 {
        if self.byte_pos >= self.data.len() {
            return 0;
        }
        let bit = (self.data[self.byte_pos] >> self.bit_pos) & 1;
        self.bit_pos += 1;
        if self.bit_pos == 8 {
            self.byte_pos += 1;
            self.bit_pos = 0;
        }
        bit
    }

    /// Read a full byte.  If the stream is currently byte-aligned the byte is
    /// returned directly; otherwise 8 individual bits are assembled.
    pub fn read_byte(&mut self) -> u8 {
        if self.bit_pos == 0 {
            if self.byte_pos >= self.data.len() {
                return 0;
            }
            let b = self.data[self.byte_pos];
            self.byte_pos += 1;
            b
        } else {
            let mut value: u8 = 0;
            for i in 0..8 {
                value |= self.read_bit() << i;
            }
            value
        }
    }

    /// Returns `true` when all bits have been consumed.
    pub fn exhausted(&self) -> bool {
        self.byte_pos >= self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_single_bits_lsb_first() {
        let mut bs = BitOutputStream::new();
        bs.write_bit(1); // bit 0
        bs.write_bit(0); // bit 1
        bs.write_bit(1); // bit 2
        bs.write_bit(1); // bit 3
        bs.write_bit(0); // bit 4
        bs.write_bit(0); // bit 5
        bs.write_bit(0); // bit 6
        bs.write_bit(0); // bit 7
        let bytes = bs.finish();
        assert_eq!(bytes, vec![0b00001101]); // 0x0D
    }

    #[test]
    fn write_byte_aligned() {
        let mut bs = BitOutputStream::new();
        bs.write_byte(0xAB);
        let bytes = bs.finish();
        assert_eq!(bytes, vec![0xAB]);
    }

    #[test]
    fn write_byte_unaligned() {
        let mut bs = BitOutputStream::new();
        bs.write_bit(1); // bit 0 of byte 0
        bs.write_byte(0xFF); // spans bytes 0 and 1
        let bytes = bs.finish();
        // byte 0: bit0=1, bits1-7=1111111 = 0xFF
        // byte 1: bit0=1, bits1-7=0000000 = 0x01
        assert_eq!(bytes.len(), 2);
        assert_eq!(bytes[0], 0xFF);
        assert_eq!(bytes[1], 0x01);
    }

    #[test]
    fn write_multiple_bytes() {
        let mut bs = BitOutputStream::new();
        bs.write_bytes(&[0x01, 0x02, 0x03]);
        let bytes = bs.finish();
        assert_eq!(bytes, vec![0x01, 0x02, 0x03]);
    }

    #[test]
    fn partial_byte_flush() {
        let mut bs = BitOutputStream::new();
        bs.write_bit(1);
        bs.write_bit(1);
        bs.write_bit(0);
        let bytes = bs.finish();
        assert_eq!(bytes, vec![0b00000011]); // bits 0,1 set, rest zero-padded
    }

    #[test]
    fn read_bits_match_written() {
        let mut out = BitOutputStream::new();
        out.write_bit(1);
        out.write_bit(0);
        out.write_bit(1);
        out.write_byte(0x42);
        out.write_bit(1);
        let data = out.finish();

        let mut inp = BitInputStream::new(&data);
        assert_eq!(inp.read_bit(), 1);
        assert_eq!(inp.read_bit(), 0);
        assert_eq!(inp.read_bit(), 1);
        assert_eq!(inp.read_byte(), 0x42);
        assert_eq!(inp.read_bit(), 1);
    }

    #[test]
    fn read_byte_aligned() {
        let data = vec![0xAB, 0xCD];
        let mut inp = BitInputStream::new(&data);
        assert_eq!(inp.read_byte(), 0xAB);
        assert_eq!(inp.read_byte(), 0xCD);
        assert!(inp.exhausted());
    }

    #[test]
    fn read_past_end_returns_zero() {
        let data = vec![0xFF];
        let mut inp = BitInputStream::new(&data);
        assert_eq!(inp.read_byte(), 0xFF);
        assert_eq!(inp.read_bit(), 0); // past end
        assert_eq!(inp.read_byte(), 0); // past end
    }
}
