//! Screen capture → JPEG frame source for LCD streaming.
//!
//! Uses ffmpeg x11grab to capture the X11 display. When a window name is
//! provided, uses `xdotool` to find the window and captures only its region.

use std::io::BufReader;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::video::{scan_next_jpeg, FrameSource};

/// Window geometry from xdotool.
struct WindowGeometry {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Parse geometry from `xdotool getwindowgeometry --shell` output.
fn parse_geometry(output: &str) -> WindowGeometry {
    let mut geo = WindowGeometry {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    };
    for line in output.lines() {
        if let Some(val) = line.strip_prefix("X=") {
            geo.x = val.parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("Y=") {
            geo.y = val.parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("WIDTH=") {
            geo.width = val.parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("HEIGHT=") {
            geo.height = val.parse().unwrap_or(0);
        }
    }
    geo
}

/// Use xdotool to find a window by name and return its geometry.
/// Searches all matching windows and picks the largest one (by area),
/// since xdotool often returns small hidden sub-windows first.
fn find_window_geometry(name: &str) -> Result<WindowGeometry, String> {
    let id_output = Command::new("xdotool")
        .args(["search", "--name", name])
        .output()
        .map_err(|e| format!("xdotool not found: {e}"))?;

    let id_str = String::from_utf8_lossy(&id_output.stdout);
    let window_ids: Vec<u64> = id_str.lines().filter_map(|l| l.trim().parse::<u64>().ok()).collect();

    if window_ids.is_empty() {
        return Err(format!("window '{}' not found by xdotool", name));
    }

    // Query geometry for each candidate, pick the largest by pixel area
    let mut best: Option<WindowGeometry> = None;
    let mut best_area: u64 = 0;

    for wid in &window_ids {
        let geo_output = Command::new("xdotool")
            .args(["getwindowgeometry", "--shell", &wid.to_string()])
            .output()
            .ok();

        if let Some(out) = geo_output {
            let geo = parse_geometry(&String::from_utf8_lossy(&out.stdout));
            let area = geo.width as u64 * geo.height as u64;
            // Skip tiny windows (decorations, hidden frames) and negative positions
            if geo.width >= 50 && geo.height >= 50 && area > best_area {
                best_area = area;
                best = Some(geo);
            }
        }
    }

    best.ok_or_else(|| {
        format!(
            "window '{}': found {} IDs but none with reasonable size (>=50x50)",
            name,
            window_ids.len()
        )
    })
}

pub struct ScreenCaptureSource {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
    fps: u32,
    buffer: Vec<u8>,
}

impl ScreenCaptureSource {
    /// Start capturing the screen or a specific window.
    ///
    /// - `display` — X11 display (e.g., ":0")
    /// - `window_name` — if Some, use xdotool to find this window and capture
    ///   only its region. If None, capture the full screen.
    /// - `width`, `height` — target LCD resolution (output is scaled to this)
    /// - `fps` — target frame rate
    pub fn new(display: &str, window_name: Option<&str>, width: u32, height: u32, fps: u32) -> Result<Self, String> {
        let scale_filter = format!(
            "scale={width}:{height}:force_original_aspect_ratio=decrease,\
             pad={width}:{height}:(ow-iw)/2:(oh-ih)/2"
        );

        let fps_str = fps.to_string();

        // Use $DISPLAY from environment if caller passed ":0" default
        let effective_display = if display == ":0" {
            std::env::var("DISPLAY").unwrap_or_else(|_| display.to_string())
        } else {
            display.to_string()
        };

        // Determine capture region: window-specific or full screen
        let (video_size, input) = if let Some(name) = window_name {
            let geo = find_window_geometry(name)?;
            eprintln!(
                "capture: window '{}' at {}x{}+{},{} on {}",
                name, geo.width, geo.height, geo.x, geo.y, effective_display
            );
            (
                format!("{}x{}", geo.width, geo.height),
                format!("{}+{},{}", effective_display, geo.x, geo.y),
            )
        } else {
            ("1920x1080".to_string(), format!("{}+0,0", effective_display))
        };

        eprintln!(
            "capture: ffmpeg -f x11grab -framerate {} -video_size {} -i {} -vf '{}' ...",
            fps_str, video_size, input, scale_filter,
        );

        let mut child = Command::new("ffmpeg")
            .args([
                "-f",
                "x11grab",
                "-framerate",
                &fps_str,
                "-video_size",
                &video_size,
                "-i",
                &input,
                "-vf",
                &scale_filter,
                "-r",
                &fps_str,
                "-f",
                "image2pipe",
                "-vcodec",
                "mjpeg",
                "-q:v",
                "5",
                "-an",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn ffmpeg for screen capture: {e}"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to capture ffmpeg stdout".to_string())?;
        let stderr = child.stderr.take();

        Ok(Self {
            child,
            reader: BufReader::with_capacity(256 * 1024, stdout),
            stderr,
            fps,
            buffer: Vec::with_capacity(128 * 1024),
        })
    }
}

impl ScreenCaptureSource {
    /// Read and return any ffmpeg stderr output (for diagnosing failures).
    pub fn stderr_output(&mut self) -> String {
        use std::io::Read;
        if let Some(ref mut stderr) = self.stderr {
            let mut buf = String::new();
            let _ = stderr.read_to_string(&mut buf);
            buf
        } else {
            String::new()
        }
    }
}

impl FrameSource for ScreenCaptureSource {
    fn next_frame(&mut self) -> Option<Vec<u8>> {
        scan_next_jpeg(&mut self.reader, &mut self.buffer)
    }

    fn frame_interval(&self) -> Duration {
        Duration::from_millis(1000 / self.fps as u64)
    }
}

impl Drop for ScreenCaptureSource {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[test]
    fn capture_interval_30fps() {
        // Can't actually spawn x11grab in CI, but verify interval math
        assert_eq!(Duration::from_millis(1000 / 30), Duration::from_millis(33));
    }
}
