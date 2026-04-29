//! H.264 on-device playback protocol.
//!
//! The LCD firmware has a built-in H.264 decoder with 3 storage blocks.
//! Upload: split file into 202,752-byte chunks, each with a 512-byte header.
//! Playback: CMD_START_PLAY (0x79), poll CMD_QUERY_BLOCK (0x7a), CMD_STOP_PLAY (0x7b).

use crate::{CMD_PUSH_JPG, CMD_QUERY_BLOCK, CMD_START_PLAY, CMD_STOP_PLAY, MAGIC_1, MAGIC_2, PACKET_SIZE};

/// H.264 upload block size (202,752 bytes).
pub const H264_BLOCK_SIZE: usize = 202_752;

/// Maximum number of storage blocks on device.
pub const MAX_BLOCKS: usize = 3;

pub fn chunk_count(file_size: usize) -> usize {
    if file_size == 0 {
        return 0;
    }
    file_size.div_ceil(H264_BLOCK_SIZE)
}

pub struct H264Upload {
    pub data: Vec<u8>,
    pub chunks: usize,
}

impl H264Upload {
    pub fn from_file(path: &std::path::Path) -> Result<Self, String> {
        let data = std::fs::read(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        if data.is_empty() {
            return Err("H.264 file is empty".into());
        }
        let chunks = chunk_count(data.len());
        Ok(Self { data, chunks })
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Self, String> {
        if data.is_empty() {
            return Err("H.264 data is empty".into());
        }
        let chunks = chunk_count(data.len());
        Ok(Self { data, chunks })
    }

    pub fn chunk(&self, index: usize) -> Option<&[u8]> {
        let start = index * H264_BLOCK_SIZE;
        if start >= self.data.len() {
            return None;
        }
        let end = (start + H264_BLOCK_SIZE).min(self.data.len());
        Some(&self.data[start..end])
    }
}

/// Build 512-byte header for an H.264 upload chunk.
/// Layout: byte[0]=cmd, [2]=magic1, [3]=magic2,
/// [4..8]=timestamp_be, [8..12]=chunk_size_be, [12..16]=total_size_be,
/// [16]=block, [17]=chunk_index.
pub fn build_upload_header(
    block: u8,
    chunk_index: u8,
    chunk_size: u32,
    total_size: u32,
    timestamp_ms: u64,
) -> [u8; PACKET_SIZE] {
    let mut h = [0u8; PACKET_SIZE];
    h[0] = CMD_PUSH_JPG;
    h[2] = MAGIC_1;
    h[3] = MAGIC_2;
    h[4..8].copy_from_slice(&(timestamp_ms as u32).to_be_bytes());
    h[8..12].copy_from_slice(&chunk_size.to_be_bytes());
    h[12..16].copy_from_slice(&total_size.to_be_bytes());
    h[16] = block;
    h[17] = chunk_index;
    h
}

pub fn build_start_play(block: u8) -> [u8; PACKET_SIZE] {
    let mut p = [0u8; PACKET_SIZE];
    p[0] = CMD_START_PLAY;
    p[2] = MAGIC_1;
    p[3] = MAGIC_2;
    p[8] = block;
    p
}

pub fn build_stop_play() -> [u8; PACKET_SIZE] {
    let mut p = [0u8; PACKET_SIZE];
    p[0] = CMD_STOP_PLAY;
    p[2] = MAGIC_1;
    p[3] = MAGIC_2;
    p
}

pub fn build_query_block() -> [u8; PACKET_SIZE] {
    let mut p = [0u8; PACKET_SIZE];
    p[0] = CMD_QUERY_BLOCK;
    p[2] = MAGIC_1;
    p[3] = MAGIC_2;
    p
}

/// Parse query_block response. Returns free space per block [0..3].
/// Response bytes[8], [9], [10] indicate pending data in blocks 0, 1, 2.
/// 0 = block consumed, >0 = data remaining.
pub fn parse_block_status(response: &[u8]) -> [u8; MAX_BLOCKS] {
    let mut status = [0u8; MAX_BLOCKS];
    if response.len() > 10 {
        status[0] = response[8];
        status[1] = response[9];
        status[2] = response[10];
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_count_exact() {
        assert_eq!(chunk_count(H264_BLOCK_SIZE), 1);
        assert_eq!(chunk_count(H264_BLOCK_SIZE * 3), 3);
    }

    #[test]
    fn chunk_count_partial() {
        assert_eq!(chunk_count(H264_BLOCK_SIZE + 1), 2);
        assert_eq!(chunk_count(1), 1);
    }

    #[test]
    fn chunk_count_zero() {
        assert_eq!(chunk_count(0), 0);
    }

    #[test]
    fn upload_chunks_cover_full_data() {
        let data = vec![0u8; H264_BLOCK_SIZE * 2 + 100];
        let upload = H264Upload::from_bytes(data.clone()).unwrap();
        assert_eq!(upload.chunks, 3);
        assert_eq!(upload.chunk(0).unwrap().len(), H264_BLOCK_SIZE);
        assert_eq!(upload.chunk(1).unwrap().len(), H264_BLOCK_SIZE);
        assert_eq!(upload.chunk(2).unwrap().len(), 100);
        assert!(upload.chunk(3).is_none());
    }

    #[test]
    fn upload_header_has_correct_command() {
        let h = build_upload_header(0, 1, 1000, 500000, 1234567890);
        assert_eq!(h[0], CMD_PUSH_JPG);
        assert_eq!(h[2], MAGIC_1);
        assert_eq!(h[3], MAGIC_2);
        assert_eq!(h[16], 0); // block
        assert_eq!(h[17], 1); // chunk_index
    }

    #[test]
    fn start_play_packet() {
        let p = build_start_play(2);
        assert_eq!(p[0], CMD_START_PLAY);
        assert_eq!(p[8], 2);
    }

    #[test]
    fn stop_play_packet() {
        let p = build_stop_play();
        assert_eq!(p[0], CMD_STOP_PLAY);
    }

    #[test]
    fn query_block_packet() {
        let p = build_query_block();
        assert_eq!(p[0], CMD_QUERY_BLOCK);
    }

    #[test]
    fn parse_block_status_extracts_bytes() {
        let mut resp = vec![0u8; 16];
        resp[8] = 5;
        resp[9] = 0;
        resp[10] = 3;
        let status = parse_block_status(&resp);
        assert_eq!(status, [5, 0, 3]);
    }

    #[test]
    fn parse_block_status_short_response() {
        let resp = vec![0u8; 5]; // too short
        let status = parse_block_status(&resp);
        assert_eq!(status, [0, 0, 0]);
    }

    #[test]
    fn empty_data_rejected() {
        assert!(H264Upload::from_bytes(vec![]).is_err());
    }
}
