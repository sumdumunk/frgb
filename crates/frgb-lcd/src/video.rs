//! Video/image frame sources for LCD streaming.

use std::io::{BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// A source of JPEG frames for LCD streaming.
pub trait FrameSource: Send {
    /// Next JPEG frame, or None when stream ends.
    fn next_frame(&mut self) -> Option<Vec<u8>>;
    /// Target duration between frames.
    fn frame_interval(&self) -> Duration;
}

/// Scan a buffered reader for the next complete JPEG frame (SOI 0xFFD8 → EOI 0xFFD9).
/// Returns the JPEG bytes, or `None` on EOF.
pub fn scan_next_jpeg<R: Read>(reader: &mut BufReader<R>, buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    buffer.clear();
    let mut byte_buf = [0u8; 1];
    let mut prev: u8 = 0;
    let mut in_frame = false;

    loop {
        if reader.read_exact(&mut byte_buf).is_err() {
            return if in_frame && buffer.len() > 2 {
                Some(std::mem::take(buffer))
            } else {
                None
            };
        }
        let b = byte_buf[0];

        if !in_frame {
            if prev == 0xFF && b == 0xD8 {
                in_frame = true;
                buffer.push(0xFF);
                buffer.push(0xD8);
            }
            prev = b;
            continue;
        }

        buffer.push(b);

        if prev == 0xFF && b == 0xD9 {
            return Some(std::mem::take(buffer));
        }
        prev = b;
    }
}

/// Streams JPEG frames from a video file via ffmpeg subprocess.
///
/// Spawns:
/// `ffmpeg -i <path> -vf scale=<w>:<h>:force_original_aspect_ratio=decrease,pad=... -r <fps> -f image2pipe -vcodec mjpeg -q:v <quality> -`
///
/// Reads JPEG frames from stdout by scanning for SOI (0xFFD8) / EOI (0xFFD9) markers.
pub struct FfmpegFrameSource {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
    fps: u32,
    buffer: Vec<u8>,
}

impl FfmpegFrameSource {
    /// Spawn an ffmpeg process and start reading JPEG frames from it.
    ///
    /// - `path`    — path to the input file (video, GIF, image)
    /// - `width`   — target frame width in pixels
    /// - `height`  — target frame height in pixels
    /// - `fps`     — target frames per second
    /// - `quality` — MJPEG quality scale (1 = best, 31 = worst; 5 is a good default)
    pub fn new(path: &str, width: u32, height: u32, fps: u32, quality: u32) -> Result<Self, String> {
        let scale_filter = format!(
            "scale={width}:{height}:force_original_aspect_ratio=decrease,\
             pad={width}:{height}:(ow-iw)/2:(oh-ih)/2"
        );

        let mut child = Command::new("ffmpeg")
            .args([
                "-i",
                path,
                "-vf",
                &scale_filter,
                "-r",
                &fps.to_string(),
                "-f",
                "image2pipe",
                "-vcodec",
                "mjpeg",
                "-q:v",
                &quality.to_string(),
                "-an", // no audio
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to spawn ffmpeg: {e}"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to capture ffmpeg stdout".to_string())?;

        Ok(Self {
            child,
            reader: BufReader::with_capacity(256 * 1024, stdout),
            fps,
            buffer: Vec::with_capacity(128 * 1024),
        })
    }
}

impl FrameSource for FfmpegFrameSource {
    fn next_frame(&mut self) -> Option<Vec<u8>> {
        scan_next_jpeg(&mut self.reader, &mut self.buffer)
    }

    fn frame_interval(&self) -> Duration {
        Duration::from_millis(1000 / self.fps as u64)
    }
}

impl Drop for FfmpegFrameSource {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_returns_error_for_nonexistent_file() {
        // ffmpeg will either fail to spawn (not installed) or exit immediately
        // with an error because the input file doesn't exist. Either way,
        // next_frame() should return None (no frames from a missing file).
        // We test the construction path here: if ffmpeg isn't installed we get
        // an Err; if it is installed it spawns but produces no frames.
        let result = FfmpegFrameSource::new("/tmp/__frgb_test_nonexistent_file_xyz__.mp4", 400, 400, 24, 5);

        match result {
            Err(e) => {
                // ffmpeg not installed — acceptable
                assert!(e.contains("ffmpeg"), "unexpected error: {e}");
            }
            Ok(mut src) => {
                // ffmpeg spawned but should produce no frames for a missing file
                let frame = src.next_frame();
                assert!(frame.is_none(), "expected no frames for nonexistent file, got Some");
            }
        }
    }
}
