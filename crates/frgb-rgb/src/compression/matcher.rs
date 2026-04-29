//! Greedy LZ77 match finder using hash chains.
//!
//! Searches for the longest backward match within a sliding dictionary window.
//! Minimum match length is 2 bytes (TUZ requirement).

/// Minimum match length required by TUZ format.
pub const TUZ_MIN_MATCH_LEN: usize = 2;

/// Default dictionary size (32 KB, used for v1 with dict_bits=15).
const DEFAULT_DICT_SIZE: usize = 1 << 15;

/// Hash table size — must be a power of two.
const HASH_SIZE: usize = 1 << 16;

/// Maximum chain length to traverse before giving up.
const MAX_CHAIN_LEN: u32 = 256;

/// Represents an LZ77 match result.
#[derive(Debug, Clone, Copy)]
pub struct Match {
    /// Backward distance from current position (1-based).
    pub distance: usize,
    /// Match length in bytes.
    pub length: usize,
}

impl Match {
    pub const NONE: Match = Match { distance: 0, length: 0 };

    /// Returns `true` if this is a valid match (length >= MIN_MATCH_LEN).
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.distance > 0 && self.length >= TUZ_MIN_MATCH_LEN
    }
}

/// Hash-chain based LZ77 match finder.
///
/// Uses a hash table indexed by 3-byte trigrams to find candidate match
/// positions, then walks the chain to find the longest match within the
/// dictionary window.
pub struct MatchFinder<'a> {
    data: &'a [u8],
    dict_size: usize,
    /// head[hash] = most recent position with that hash (or u32::MAX if none).
    head: Vec<u32>,
    /// prev[pos % dict_size] = previous position in the chain with the same hash.
    prev: Vec<u32>,
}

impl<'a> MatchFinder<'a> {
    /// Create a new match finder for the given data.
    ///
    /// `dict_bits` controls the dictionary window size: window = 2^dict_bits.
    /// Default is 15 (32 KB).
    pub fn new(data: &'a [u8], dict_bits: Option<u32>) -> Self {
        let dict_size = match dict_bits {
            Some(b) => 1usize << b,
            None => DEFAULT_DICT_SIZE,
        };
        let prev_size = dict_size.min(data.len()).max(1);
        Self {
            data,
            dict_size,
            head: vec![u32::MAX; HASH_SIZE],
            prev: vec![u32::MAX; prev_size],
        }
    }

    /// Insert position `pos` into the hash chain.
    fn insert(&mut self, pos: usize) {
        if pos + 2 >= self.data.len() {
            return;
        }
        let h = Self::hash3(self.data, pos);
        let prev_head = self.head[h];
        self.prev[pos % self.dict_size] = prev_head;
        self.head[h] = pos as u32;
    }

    /// Find the best (longest) match at `pos`, inserting all skipped
    /// positions into the hash chain along the way.
    ///
    /// This is the main entry point for greedy parsing. Call it sequentially
    /// for each position; internally it inserts every position it sees.
    pub fn find_best_match(&mut self, pos: usize) -> Match {
        // Insert current position first
        self.insert(pos);

        if pos == 0 || pos + TUZ_MIN_MATCH_LEN > self.data.len() {
            return Match::NONE;
        }

        // Need at least 3 bytes for the hash
        if pos + 2 >= self.data.len() {
            return Match::NONE;
        }

        let h = Self::hash3(self.data, pos);
        let min_pos = pos.saturating_sub(self.dict_size);
        let max_len = self.data.len() - pos;

        let mut best = Match::NONE;
        let mut chain_pos = self.head[h];
        let mut chain_steps: u32 = 0;

        while chain_pos != u32::MAX && chain_steps < MAX_CHAIN_LEN {
            let cp = chain_pos as usize;

            // Skip self and forward references
            if cp >= pos {
                chain_pos = self.prev[cp % self.dict_size];
                chain_steps += 1;
                continue;
            }

            // Outside dictionary window
            if cp < min_pos {
                break;
            }

            // Compute match length
            let len = self.match_length(cp, pos, max_len);
            if len >= TUZ_MIN_MATCH_LEN {
                let dist = pos - cp;
                if len > best.length || (len == best.length && dist < best.distance) {
                    best = Match {
                        distance: dist,
                        length: len,
                    };
                    // Can't do better than max_len
                    if len == max_len {
                        break;
                    }
                }
            }

            chain_pos = self.prev[cp % self.dict_size];
            chain_steps += 1;
        }

        best
    }

    /// Compute the byte-by-byte match length between two positions.
    fn match_length(&self, a: usize, b: usize, max_len: usize) -> usize {
        let mut i = 0;
        while i < max_len && self.data[a + i] == self.data[b + i] {
            i += 1;
        }
        i
    }

    /// 3-byte hash function.
    #[inline]
    fn hash3(data: &[u8], pos: usize) -> usize {
        let b0 = data[pos] as usize;
        let b1 = data[pos + 1] as usize;
        let b2 = data[pos + 2] as usize;
        ((b0 << 10) ^ (b1 << 5) ^ b2) & (HASH_SIZE - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_match_at_start() {
        let data = b"abcdefgh";
        let mut mf = MatchFinder::new(data, None);
        let m = mf.find_best_match(0);
        assert!(!m.is_valid());
    }

    #[test]
    fn finds_repeated_pattern() {
        let data = b"abcabc";
        let mut mf = MatchFinder::new(data, None);
        // Insert and scan sequentially
        for i in 0..3 {
            mf.find_best_match(i);
        }
        let m = mf.find_best_match(3);
        assert!(m.is_valid());
        assert_eq!(m.distance, 3);
        assert_eq!(m.length, 3);
    }

    #[test]
    fn no_match_for_unique_bytes() {
        let data: Vec<u8> = (0..=255).collect();
        let mut mf = MatchFinder::new(&data, None);
        for i in 0..data.len() {
            let m = mf.find_best_match(i);
            assert!(!m.is_valid());
        }
    }

    #[test]
    fn finds_long_run() {
        let data = vec![0xAAu8; 100];
        let mut mf = MatchFinder::new(&data, None);
        mf.find_best_match(0);
        let m = mf.find_best_match(1);
        assert!(m.is_valid());
        assert_eq!(m.distance, 1);
        assert!(m.length >= 2);
    }
}
