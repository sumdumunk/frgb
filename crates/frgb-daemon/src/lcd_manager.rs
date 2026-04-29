use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::DynamicImage;

use frgb_model::device::DeviceId;
use frgb_model::lcd::{LcdConfig, LcdContent, LcdPreset, LcdPresetCategory, LcdRotation, LcdSensorStyle};
use frgb_model::GroupId;

// ── Constants ─────────────────────────────────────────────────────────────────

const SENSOR_REFRESH: Duration = Duration::from_secs(1);
const CAROUSEL_SWITCH: Duration = Duration::from_secs(3);
const GRAPH_HISTORY_LEN: usize = 60;

// ── Public types ──────────────────────────────────────────────────────────────

pub struct LcdManager {
    devices: HashMap<DeviceId, LcdDeviceState>,
    /// Ordered list of LCD device IDs (index matches LCD backend order).
    device_ids: Vec<DeviceId>,
    /// Maps fan group ID → LCD device ID(s) for that group.
    group_map: HashMap<GroupId, Vec<DeviceId>>,
}

#[derive(Debug)]
pub enum LcdAction {
    SendFrame {
        device_id: DeviceId,
        jpeg: Vec<u8>,
    },
    SetClock {
        device_id: DeviceId,
    },
    SetBrightness {
        device_id: DeviceId,
        brightness: frgb_model::Brightness,
    },
    SetRotation {
        device_id: DeviceId,
        rotation: LcdRotation,
    },
}

// ── Private types ─────────────────────────────────────────────────────────────

/// Avoids cloning a `DynamicImage` in the GIF/Preset path: borrows directly
/// from `GifState::frames` instead of copying ~900 KB per tick.
enum FrameSource<'a> {
    Borrowed(&'a DynamicImage),
    Owned(DynamicImage),
}

impl<'a> FrameSource<'a> {
    fn as_ref(&self) -> &DynamicImage {
        match self {
            FrameSource::Borrowed(img) => img,
            FrameSource::Owned(img) => img,
        }
    }
}

// ── Private state ─────────────────────────────────────────────────────────────

struct LcdDeviceState {
    config: Option<LcdConfig>,
    pending_brightness: Option<frgb_model::Brightness>,
    pending_rotation: Option<LcdRotation>,
    resolution: (u32, u32),
    last_frame_hash: u64,
    next_refresh: Instant,
    gif_state: Option<GifState>,
    capture_source: Option<Box<dyn frgb_lcd::video::FrameSource>>,
    carousel_index: usize,
    carousel_next_switch: Instant,
    graph_history: HashMap<String, VecDeque<f32>>,
    template_state: Option<frgb_lcd_render::template::TemplateState>,
}

struct GifState {
    frames: Vec<DynamicImage>,
    fps: u8,
    index: usize,
}

// ── LcdManager implementation ─────────────────────────────────────────────────

impl LcdManager {
    pub fn new() -> Self {
        LcdManager {
            devices: HashMap::new(),
            device_ids: Vec::new(),
            group_map: HashMap::new(),
        }
    }

    /// Map a fan group to LCD device ID(s).
    /// Called after discovery when we know which groups have LCD fans.
    pub fn map_group(&mut self, group_id: GroupId, lcd_device_id: DeviceId) {
        let ids = self.group_map.entry(group_id).or_default();
        if !ids.contains(&lcd_device_id) {
            ids.push(lcd_device_id);
        }
    }

    /// Number of registered LCD devices.
    pub fn device_count(&self) -> usize {
        self.device_ids.len()
    }

    /// Look up LCD device IDs for a fan group.
    #[allow(dead_code)]
    pub fn lcd_ids_for_group(&self, group_id: GroupId) -> Vec<DeviceId> {
        self.group_map.get(&group_id).cloned().unwrap_or_default()
    }

    /// Set config for all LCD devices mapped to a fan group.
    #[allow(dead_code)]
    pub fn set_config_for_group(&mut self, group_id: GroupId, config: LcdConfig) {
        let ids = self.lcd_ids_for_group(group_id);
        for id in ids {
            self.set_config(id, config.clone());
        }
    }

    /// Look up LCD DeviceId by index (matches LCD backend order).
    pub fn device_id_by_index(&self, index: u8) -> Option<DeviceId> {
        self.device_ids.get(index as usize).copied()
    }

    pub fn register_device(&mut self, id: DeviceId, width: u32, height: u32) {
        if !self.device_ids.contains(&id) {
            self.device_ids.push(id);
        }
        self.devices.entry(id).or_insert_with(|| LcdDeviceState {
            config: None,
            pending_brightness: None,
            pending_rotation: None,
            resolution: (width, height),
            last_frame_hash: 0,
            next_refresh: Instant::now(),
            gif_state: None,
            capture_source: None,
            carousel_index: 0,
            carousel_next_switch: Instant::now(),
            graph_history: HashMap::new(),
            template_state: None,
        });
    }

    pub fn set_config(&mut self, device_id: DeviceId, config: LcdConfig) {
        if let Some(state) = self.devices.get_mut(&device_id) {
            state.pending_brightness = Some(config.brightness);
            state.pending_rotation = Some(config.rotation);

            // Clear any previous capture source
            state.capture_source = None;

            // Initialize screen capture if content is ScreenCapture
            if let LcdContent::ScreenCapture {
                ref display,
                ref window,
                fps,
            } = config.content
            {
                let disp = display.as_deref().unwrap_or(":0");
                let win = window.as_deref();
                let (w, h) = state.resolution;
                match frgb_lcd::capture::ScreenCaptureSource::new(disp, win, w, h, fps as u32) {
                    Ok(src) => {
                        state.capture_source = Some(Box::new(src));
                        tracing::info!("lcd_manager: screen capture started for {:?}", device_id);
                    }
                    Err(e) => tracing::warn!("lcd_manager: screen capture failed: {e}"),
                }
            }

            // Decode GIF if content is Gif
            state.gif_state = match &config.content {
                LcdContent::Gif { frames, fps } => {
                    let fps_val = *fps;
                    if frames.len() == 1 {
                        // Single blob — try animated GIF decode
                        use image::codecs::gif::GifDecoder;
                        use image::AnimationDecoder;
                        use std::io::Cursor;
                        match GifDecoder::new(Cursor::new(&frames[0])) {
                            Ok(decoder) => {
                                let decoded: Vec<DynamicImage> = decoder
                                    .into_frames()
                                    .filter_map(|f| f.ok())
                                    .map(|f| DynamicImage::ImageRgba8(f.into_buffer()))
                                    .collect();
                                if decoded.is_empty() {
                                    None
                                } else {
                                    Some(GifState {
                                        frames: decoded,
                                        fps: if fps_val == 0 { 24 } else { fps_val },
                                        index: 0,
                                    })
                                }
                            }
                            Err(_) => {
                                // Not an animated GIF — try as static image
                                image::load_from_memory(&frames[0]).ok().map(|img| GifState {
                                    frames: vec![img],
                                    fps: 1,
                                    index: 0,
                                })
                            }
                        }
                    } else {
                        // Multiple pre-decoded frame blobs
                        let decoded: Vec<DynamicImage> =
                            frames.iter().filter_map(|f| image::load_from_memory(f).ok()).collect();
                        if decoded.is_empty() {
                            None
                        } else {
                            Some(GifState {
                                frames: decoded,
                                fps: if fps_val == 0 { 24 } else { fps_val },
                                index: 0,
                            })
                        }
                    }
                }
                _ => None,
            };

            // Reset carousel, graph, and template state
            state.template_state = None;
            state.carousel_index = 0;
            state.carousel_next_switch = Instant::now();
            state.graph_history.clear();

            // Force immediate render on next tick
            state.next_refresh = Instant::now();
            state.last_frame_hash = 0;

            state.config = Some(config);
        }
    }

    pub fn tick(&mut self, sensors: &HashMap<String, f32>) -> Vec<LcdAction> {
        let now = Instant::now();
        let mut actions: Vec<LcdAction> = Vec::new();

        for (device_id, state) in &mut self.devices {
            // Emit pending brightness
            if let Some(b) = state.pending_brightness.take() {
                actions.push(LcdAction::SetBrightness {
                    device_id: *device_id,
                    brightness: b,
                });
            }

            // Emit pending rotation
            if let Some(r) = state.pending_rotation.take() {
                actions.push(LcdAction::SetRotation {
                    device_id: *device_id,
                    rotation: r,
                });
            }

            let config = match &state.config {
                Some(c) => c,
                None => continue,
            };

            let (width, height) = state.resolution;

            // Clock: emit SetClock on refresh (resync every 60s)
            if matches!(config.content, LcdContent::Clock(_)) {
                if now >= state.next_refresh {
                    actions.push(LcdAction::SetClock { device_id: *device_id });
                    state.next_refresh = now + Duration::from_secs(60);
                }
                continue;
            }

            // ScreenCapture: raw JPEG bypass — skip image→jpeg encode path
            if matches!(config.content, LcdContent::ScreenCapture { .. }) {
                if now >= state.next_refresh {
                    if let Some(ref mut src) = state.capture_source {
                        if let Some(jpeg) = src.next_frame() {
                            let h = hash_bytes(&jpeg);
                            if h != state.last_frame_hash {
                                state.last_frame_hash = h;
                                actions.push(LcdAction::SendFrame {
                                    device_id: *device_id,
                                    jpeg,
                                });
                            }
                            state.next_refresh = now + src.frame_interval();
                        }
                    }
                }
                continue;
            }

            // Skip if before next_refresh
            if now < state.next_refresh {
                continue;
            }

            // Render frame based on content type
            let img_opt: Option<FrameSource<'_>> = match &config.content {
                LcdContent::Off => Some(FrameSource::Owned(frgb_lcd_render::black_frame(width, height))),

                LcdContent::Text(_) | LcdContent::Image(_) | LcdContent::SystemInfo => Some(FrameSource::Owned(
                    frgb_lcd_render::render(&config.content, sensors, width, height),
                )),

                LcdContent::Sensor(display) => {
                    if display.style == LcdSensorStyle::Graph {
                        // Update graph history ring buffer
                        let key = sensor_key(&display.sensor);
                        let label = display.label.as_deref().unwrap_or(key).to_string();
                        let entry = state.graph_history.entry(key.to_string()).or_insert_with(VecDeque::new);
                        if let Some(&val) = sensors.get(key) {
                            entry.push_back(val);
                            if entry.len() > GRAPH_HISTORY_LEN {
                                entry.pop_front();
                            }
                        }
                        let history_slice: Vec<f32> = entry.iter().copied().collect();
                        let current = sensors.get(key).copied();
                        let color = frgb_lcd_render::color_from_sensor_color(display.color);
                        Some(FrameSource::Owned(frgb_lcd_render::sensor::render_graph_with_history(
                            &label,
                            &history_slice,
                            current,
                            &display.unit,
                            color,
                            width,
                            height,
                        )))
                    } else {
                        Some(FrameSource::Owned(frgb_lcd_render::render(
                            &config.content,
                            sensors,
                            width,
                            height,
                        )))
                    }
                }

                LcdContent::SensorCarousel(displays) => {
                    if displays.is_empty() {
                        Some(FrameSource::Owned(frgb_lcd_render::black_frame(width, height)))
                    } else {
                        // Advance carousel index on timer
                        if now >= state.carousel_next_switch {
                            state.carousel_index = (state.carousel_index + 1) % displays.len();
                            state.carousel_next_switch = now + CAROUSEL_SWITCH;
                        }
                        let display = &displays[state.carousel_index];
                        let sensor_content = LcdContent::Sensor(display.clone());
                        Some(FrameSource::Owned(frgb_lcd_render::render(
                            &sensor_content,
                            sensors,
                            width,
                            height,
                        )))
                    }
                }

                LcdContent::Gif { .. } | LcdContent::Preset(_) => {
                    // Advance the frame index first (mutable borrow), then borrow
                    // the frame immutably. Two separate borrows of gif_state avoid
                    // a clone of the ~900 KB DynamicImage on every tick.
                    let frame_idx = state.gif_state.as_mut().map(|gif| {
                        let idx = gif.index;
                        gif.index = (idx + 1) % gif.frames.len();
                        idx
                    });
                    match (frame_idx, state.gif_state.as_ref()) {
                        (Some(idx), Some(gif)) => Some(FrameSource::Borrowed(&gif.frames[idx])),
                        _ => Some(FrameSource::Owned(frgb_lcd_render::black_frame(width, height))),
                    }
                }

                LcdContent::Template(ref tmpl) => {
                    if state.template_state.is_none() {
                        state.template_state = Some(frgb_lcd_render::template::TemplateState::new(tmpl));
                    }
                    let ts = state.template_state.as_ref().unwrap();
                    if ts.needs_render(tmpl, sensors) {
                        let img = frgb_lcd_render::template::render_template(tmpl, sensors, width, height);
                        state.template_state.as_mut().unwrap().mark_rendered(tmpl, sensors);
                        Some(FrameSource::Owned(img))
                    } else {
                        None
                    }
                }

                LcdContent::Clock(_) | LcdContent::Video(_) | LcdContent::ScreenCapture { .. } => None,
            };

            let frame = match img_opt {
                Some(f) => f,
                None => continue,
            };

            // JPEG encode
            let jpeg = match frgb_lcd::jpeg::prepare_jpeg(frame.as_ref(), width, height, 85) {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!("lcd_manager: JPEG encode failed for {:?}: {e}", device_id);
                    continue;
                }
            };

            // Hash dedup — skip USB push if content unchanged
            let h = hash_bytes(&jpeg);
            if h == state.last_frame_hash {
                // Still advance next_refresh so we re-check timing
                state.next_refresh = now + refresh_interval(&config.content, &state.gif_state);
                continue;
            }
            state.last_frame_hash = h;

            actions.push(LcdAction::SendFrame {
                device_id: *device_id,
                jpeg,
            });

            // Set next_refresh
            state.next_refresh = now + refresh_interval(&config.content, &state.gif_state);
        }

        actions
    }

    pub fn has_devices(&self) -> bool {
        !self.devices.is_empty()
    }

    /// Create preset category subdirectories under `~/.config/frgb/presets/` if they don't exist.
    pub fn init_presets(&self) {
        let base = presets_dir();
        let categories = ["cooler", "fan", "led", "ga2v", "legacy"];
        for cat in &categories {
            let dir = base.join(cat);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::warn!("lcd_manager: failed to create preset dir {}: {e}", dir.display());
            }
        }
    }

    /// Scan `~/.config/frgb/presets/` and return all available presets.
    pub fn list_presets(&self) -> Vec<LcdPreset> {
        let base = presets_dir();
        let categories = [
            ("cooler", LcdPresetCategory::Cooler),
            ("fan", LcdPresetCategory::Fan),
            ("led", LcdPresetCategory::Led),
            ("ga2v", LcdPresetCategory::Ga2v),
            ("legacy", LcdPresetCategory::Legacy),
        ];

        let mut presets = Vec::new();
        let mut index: u8 = 0;

        for (dir_name, category) in &categories {
            let cat_dir = base.join(dir_name);
            let entries = match std::fs::read_dir(&cat_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect();
            names.sort();

            for name in names {
                let preset_dir = cat_dir.join(&name);
                let (meta_name, fps, frame_count) = parse_preset_meta(&preset_dir.join("meta.json"));
                let display_name = if meta_name.is_empty() { name.clone() } else { meta_name };
                let frames = if frame_count == 0 {
                    count_frame_files(&preset_dir)
                } else {
                    frame_count
                };
                // Load first frame as thumbnail (resize to 120px for transfer size)
                let thumbnail = load_preset_thumbnail(&preset_dir);
                presets.push(LcdPreset {
                    category: *category,
                    index,
                    name: display_name,
                    frame_count: frames,
                    fps,
                    thumbnail,
                });
                index = index.saturating_add(1);
            }
        }

        presets
    }

    /// Load frames from a preset directory into the gif_state of the given device.
    pub fn load_preset(&mut self, device_id: DeviceId, preset: &LcdPreset) {
        let base = presets_dir();
        let cat_dir_name = match preset.category {
            LcdPresetCategory::Cooler => "cooler",
            LcdPresetCategory::Fan => "fan",
            LcdPresetCategory::Led => "led",
            LcdPresetCategory::Ga2v => "ga2v",
            LcdPresetCategory::Legacy => "legacy",
        };

        // Find the preset directory by matching the display name against dir names,
        // falling back to listing all dirs in the category.
        let cat_dir = base.join(cat_dir_name);
        let preset_dir = cat_dir.join(&preset.name);

        let frames = load_frames_from_dir(&preset_dir);
        if frames.is_empty() {
            tracing::warn!(
                "lcd_manager: no frames found for preset '{}' in {}",
                preset.name,
                preset_dir.display()
            );
            return;
        }

        if let Some(state) = self.devices.get_mut(&device_id) {
            let fps = if preset.fps == 0 { 24 } else { preset.fps };
            state.gif_state = Some(GifState { frames, fps, index: 0 });
            tracing::info!(
                "lcd_manager: loaded preset '{}' ({} frames @ {}fps) for {:?}",
                preset.name,
                state.gif_state.as_ref().map(|g| g.frames.len()).unwrap_or(0),
                fps,
                device_id
            );
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns `~/.config/frgb/presets/`.
fn presets_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("frgb")
        .join("presets")
}

/// Parse meta.json and return (name, fps, frame_count). Returns defaults on error/missing.
fn parse_preset_meta(path: &PathBuf) -> (String, u8, u16) {
    let data = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return (String::new(), 24, 0),
    };
    let v: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return (String::new(), 24, 0),
    };
    let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
    let fps = v.get("fps").and_then(|f| f.as_u64()).unwrap_or(24) as u8;
    let frame_count = v.get("frame_count").and_then(|c| c.as_u64()).unwrap_or(0) as u16;
    (name, fps, frame_count)
}

/// Count jpg/jpeg/png files in a directory.
fn count_frame_files(dir: &Path) -> u16 {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("jpg") | Some("jpeg") | Some("png")
            )
        })
        .count() as u16
}

/// Load first frame from a preset directory as a thumbnail JPEG (60×60).
fn load_preset_thumbnail(dir: &Path) -> Vec<u8> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("jpg") | Some("jpeg") | Some("png")
            )
        })
        .collect();
    paths.sort();

    let first = match paths.first() {
        Some(p) => p,
        None => return Vec::new(),
    };

    // Read and resize to 60×60 thumbnail (keeps IPC payload small)
    match image::open(first) {
        Ok(img) => {
            let thumb = img.resize(60, 60, image::imageops::FilterType::Triangle);
            let mut buf = std::io::Cursor::new(Vec::new());
            if thumb.write_to(&mut buf, image::ImageFormat::Jpeg).is_ok() {
                buf.into_inner()
            } else {
                Vec::new()
            }
        }
        Err(_) => Vec::new(),
    }
}

/// Load all jpg/jpeg/png frames from a directory, sorted by filename.
fn load_frames_from_dir(dir: &Path) -> Vec<DynamicImage> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("jpg") | Some("jpeg") | Some("png")
            )
        })
        .collect();
    paths.sort();

    paths.iter().filter_map(|p| image::open(p).ok()).collect()
}

fn sensor_key(sensor: &frgb_model::sensor::Sensor) -> &'static str {
    match sensor {
        frgb_model::sensor::Sensor::Cpu => "CPU",
        frgb_model::sensor::Sensor::Gpu => "GPU",
        frgb_model::sensor::Sensor::GpuHotspot => "GPU Hotspot",
        frgb_model::sensor::Sensor::GpuVram => "GPU VRAM",
        frgb_model::sensor::Sensor::GpuPower => "GPU Power",
        frgb_model::sensor::Sensor::GpuUsage => "GPU Usage",
        frgb_model::sensor::Sensor::Water => "Water",
        frgb_model::sensor::Sensor::Motherboard { .. } => "MB",
        frgb_model::sensor::Sensor::Weighted { .. } => "CPU",
    }
}

fn refresh_interval(content: &LcdContent, gif_state: &Option<GifState>) -> Duration {
    match content {
        LcdContent::Off | LcdContent::Text(_) | LcdContent::Image(_) => Duration::from_secs(30),
        LcdContent::SystemInfo | LcdContent::Sensor(_) | LcdContent::SensorCarousel(_) => SENSOR_REFRESH,
        LcdContent::Gif { fps, .. } => {
            let effective_fps = if let Some(gs) = gif_state { gs.fps } else { *fps };
            let fps_u = effective_fps.max(1) as u64;
            Duration::from_millis(1000 / fps_u)
        }
        LcdContent::Preset(_) => {
            if let Some(gs) = gif_state {
                let fps_u = gs.fps.max(1) as u64;
                Duration::from_millis(1000 / fps_u)
            } else {
                Duration::from_millis(1000 / 24)
            }
        }
        LcdContent::Clock(_) => Duration::from_secs(60),
        LcdContent::Video(_) => Duration::from_millis(33),
        LcdContent::Template(_) => Duration::from_secs(1),
        LcdContent::ScreenCapture { fps, .. } => Duration::from_millis(1000 / (*fps).max(1) as u64),
    }
}

fn hash_bytes(data: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::lcd::{LcdConfig, LcdContent, LcdRotation, LcdSensorColor, LcdSensorDisplay, LcdSensorStyle};
    use frgb_model::sensor::{Sensor, TempUnit};

    fn test_device_id() -> DeviceId {
        DeviceId::from_vid_pid(0x1CBE, 0x0001)
    }

    fn make_manager() -> LcdManager {
        let mut mgr = LcdManager::new();
        mgr.register_device(test_device_id(), 400, 400);
        mgr
    }

    fn config(content: LcdContent) -> LcdConfig {
        LcdConfig {
            brightness: frgb_model::Brightness::new(200),
            rotation: LcdRotation::R0,
            content,
        }
    }

    fn empty_sensors() -> HashMap<String, f32> {
        HashMap::new()
    }

    // ── new_manager_is_empty ─────────────────────────────────────────────────

    #[test]
    fn new_manager_is_empty() {
        let mgr = LcdManager::new();
        assert!(!mgr.has_devices());
    }

    // ── register_device ──────────────────────────────────────────────────────

    #[test]
    fn register_device() {
        let mut mgr = LcdManager::new();
        assert!(!mgr.has_devices());
        mgr.register_device(test_device_id(), 400, 400);
        assert!(mgr.has_devices());
    }

    // ── set_config_stores_config ─────────────────────────────────────────────

    #[test]
    fn set_config_stores_config() {
        let mut mgr = make_manager();
        mgr.set_config(test_device_id(), config(LcdContent::Off));
        let state = mgr.devices.get(&test_device_id()).unwrap();
        assert!(state.config.is_some());
    }

    // ── tick_off_content_sends_black_frame_once ──────────────────────────────

    #[test]
    fn tick_off_content_sends_black_frame_once() {
        let mut mgr = make_manager();
        mgr.set_config(test_device_id(), config(LcdContent::Off));

        let actions = mgr.tick(&empty_sensors());
        let frame_count = actions
            .iter()
            .filter(|a| matches!(a, LcdAction::SendFrame { .. }))
            .count();
        assert_eq!(frame_count, 1, "first tick should send one frame");

        // second immediate tick — same content, hash unchanged → no frame
        let actions2 = mgr.tick(&empty_sensors());
        let frame_count2 = actions2
            .iter()
            .filter(|a| matches!(a, LcdAction::SendFrame { .. }))
            .count();
        assert_eq!(frame_count2, 0, "second tick with same content should not send");
    }

    // ── tick_text_sends_frame_once ───────────────────────────────────────────

    #[test]
    fn tick_text_sends_frame_once() {
        let mut mgr = make_manager();
        mgr.set_config(test_device_id(), config(LcdContent::Text("Hello".into())));

        let actions = mgr.tick(&empty_sensors());
        let frame_count = actions
            .iter()
            .filter(|a| matches!(a, LcdAction::SendFrame { .. }))
            .count();
        assert_eq!(frame_count, 1);

        // immediate second tick — hash unchanged, not past refresh → no frame
        let actions2 = mgr.tick(&empty_sensors());
        let frame_count2 = actions2
            .iter()
            .filter(|a| matches!(a, LcdAction::SendFrame { .. }))
            .count();
        assert_eq!(frame_count2, 0);
    }

    // ── tick_sensor_refreshes_after_interval ─────────────────────────────────

    #[test]
    fn tick_sensor_refreshes_after_interval() {
        let cpu_display = LcdSensorDisplay {
            sensor: Sensor::Cpu,
            label: Some("CPU".into()),
            unit: TempUnit::Celsius,
            style: LcdSensorStyle::Gauge,
            color: LcdSensorColor::Blue,
        };
        let mut mgr = make_manager();
        mgr.set_config(test_device_id(), config(LcdContent::Sensor(cpu_display)));

        let mut sensors = HashMap::new();
        sensors.insert("CPU".to_string(), 55.0f32);

        let actions = mgr.tick(&sensors);
        let frame_count = actions
            .iter()
            .filter(|a| matches!(a, LcdAction::SendFrame { .. }))
            .count();
        assert_eq!(frame_count, 1, "first tick should send");

        // Immediate second tick is before next_refresh → no frame
        let actions2 = mgr.tick(&sensors);
        let frame_count2 = actions2
            .iter()
            .filter(|a| matches!(a, LcdAction::SendFrame { .. }))
            .count();
        assert_eq!(
            frame_count2, 0,
            "immediate second tick should not send (before refresh interval)"
        );
    }

    // ── config_change_triggers_brightness_and_rotation ───────────────────────

    #[test]
    fn config_change_triggers_brightness_and_rotation() {
        let mut mgr = make_manager();
        mgr.set_config(
            test_device_id(),
            LcdConfig {
                brightness: frgb_model::Brightness::new(150),
                rotation: LcdRotation::R90,
                content: LcdContent::Off,
            },
        );

        let actions = mgr.tick(&empty_sensors());

        let b150 = frgb_model::Brightness::new(150);
        let has_brightness = actions
            .iter()
            .any(|a| matches!(a, LcdAction::SetBrightness { brightness, .. } if *brightness == b150));
        let has_rotation = actions.iter().any(|a| {
            matches!(
                a,
                LcdAction::SetRotation {
                    rotation: LcdRotation::R90,
                    ..
                }
            )
        });

        assert!(has_brightness, "should emit SetBrightness action");
        assert!(has_rotation, "should emit SetRotation action");
    }

    // ── sensor_key_maps_correctly ────────────────────────────────────────────

    #[test]
    fn sensor_key_maps_correctly() {
        use frgb_model::sensor::Sensor;
        assert_eq!(sensor_key(&Sensor::Cpu), "CPU");
        assert_eq!(sensor_key(&Sensor::Gpu), "GPU");
        assert_eq!(sensor_key(&Sensor::Water), "Water");
        assert_eq!(sensor_key(&Sensor::Motherboard { channel: 0 }), "MB");
    }

    // ── frame_hash_dedup_skips_identical_frames ───────────────────────────────

    #[test]
    fn frame_hash_dedup_skips_identical_frames() {
        let mut mgr = make_manager();
        mgr.set_config(test_device_id(), config(LcdContent::Text("Same".into())));

        // First tick sends
        let actions = mgr.tick(&empty_sensors());
        let first_sends = actions
            .iter()
            .filter(|a| matches!(a, LcdAction::SendFrame { .. }))
            .count();
        assert_eq!(first_sends, 1);

        // Force next_refresh to now so timing doesn't block
        let state = mgr.devices.get_mut(&test_device_id()).unwrap();
        state.next_refresh = Instant::now();

        // Second tick — same content, hash dedup should suppress
        let actions2 = mgr.tick(&empty_sensors());
        let second_sends = actions2
            .iter()
            .filter(|a| matches!(a, LcdAction::SendFrame { .. }))
            .count();
        assert_eq!(second_sends, 0, "identical frame hash should suppress send");
    }

    /// GIF content with a single static frame: tick produces a SendFrame, and
    /// a second tick (after forcing refresh) produces the same JPEG data since
    /// the FrameSource borrows from the same decoded frame vec.
    #[test]
    fn gif_single_frame_produces_identical_jpeg_on_retick() {
        // Create a minimal 2x2 PNG in memory to use as a single "GIF frame"
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let mut png_bytes = Vec::new();
        {
            let mut cursor = std::io::Cursor::new(&mut png_bytes);
            img.write_to(&mut cursor, image::ImageFormat::Png).unwrap();
        }

        let mut mgr = make_manager();
        mgr.set_config(
            test_device_id(),
            config(LcdContent::Gif {
                frames: vec![png_bytes],
                fps: 1,
            }),
        );

        // First tick — should produce a frame
        let actions1 = mgr.tick(&empty_sensors());
        let frame1: Vec<u8> = actions1
            .iter()
            .find_map(|a| match a {
                LcdAction::SendFrame { jpeg, .. } => Some(jpeg.clone()),
                _ => None,
            })
            .expect("first tick should produce SendFrame");

        assert!(!frame1.is_empty(), "JPEG data should be non-empty");

        // Force refresh so timing doesn't block, and reset frame hash
        // to allow re-send of the same content.
        let state = mgr.devices.get_mut(&test_device_id()).unwrap();
        state.next_refresh = Instant::now();
        state.last_frame_hash = 0;

        // Second tick — same single GIF frame, should produce identical JPEG
        let actions2 = mgr.tick(&empty_sensors());
        let frame2: Vec<u8> = actions2
            .iter()
            .find_map(|a| match a {
                LcdAction::SendFrame { jpeg, .. } => Some(jpeg.clone()),
                _ => None,
            })
            .expect("second tick should produce SendFrame after hash reset");

        assert_eq!(frame1, frame2, "identical GIF frame should produce identical JPEG data");
    }
}
