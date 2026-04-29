//! Converts LCD UI state strings to frgb-model LCD types.

use frgb_model::lcd::*;
use frgb_model::sensor::TempUnit;

use crate::convert::sensor_from_label;

pub fn rotation_from_degrees(deg: i32) -> LcdRotation {
    match deg {
        90 => LcdRotation::R90,
        180 => LcdRotation::R180,
        270 => LcdRotation::R270,
        _ => LcdRotation::R0,
    }
}

pub fn sensor_style_from_str(s: &str) -> LcdSensorStyle {
    match s {
        "Number" => LcdSensorStyle::Number,
        "Graph" => LcdSensorStyle::Graph,
        "Carousel" => LcdSensorStyle::Carousel,
        _ => LcdSensorStyle::Gauge,
    }
}

pub fn sensor_color_from_str(s: &str) -> LcdSensorColor {
    match s {
        "Green" => LcdSensorColor::Green,
        "Purple" => LcdSensorColor::Purple,
        "Red" => LcdSensorColor::Red,
        _ => LcdSensorColor::Blue,
    }
}

pub fn clock_style_from_index(idx: i32) -> ClockStyle {
    match idx {
        1 => ClockStyle::C2,
        2 => ClockStyle::C3,
        3 => ClockStyle::C4,
        4 => ClockStyle::C5,
        5 => ClockStyle::C6,
        6 => ClockStyle::C7,
        _ => ClockStyle::C1,
    }
}

/// Build an `LcdConfig` from the flat UI state values.
#[allow(clippy::too_many_arguments)]
pub fn build_lcd_config(
    content_type: &str,
    brightness: i32,
    rotation: i32,
    sensor: &str,
    style: &str,
    color: &str,
    clock_style: i32,
    text: &str,
    file_path: &str,
    presets: &[LcdPreset],
) -> LcdConfig {
    let content = match content_type {
        "Sensor" => LcdContent::Sensor(LcdSensorDisplay {
            sensor: sensor_from_label(sensor),
            label: None,
            unit: TempUnit::Celsius,
            style: sensor_style_from_str(style),
            color: sensor_color_from_str(color),
        }),
        "Clock" => LcdContent::Clock(clock_style_from_index(clock_style)),
        "System Info" => LcdContent::SystemInfo,
        "Text" => LcdContent::Text(text.to_string()),
        "Image" => {
            if !file_path.is_empty() {
                match std::fs::read(file_path) {
                    Ok(bytes) => LcdContent::Image(bytes),
                    Err(e) => {
                        tracing::warn!("Failed to read image file: {e}");
                        LcdContent::Off
                    }
                }
            } else {
                LcdContent::Off
            }
        }
        "GIF" => {
            if !file_path.is_empty() {
                match std::fs::read(file_path) {
                    // NOTE: GIF requires per-frame decoding (e.g., via image crate).
                    // Currently sends raw file bytes as a single "frame" — the daemon
                    // LCD backend must handle raw GIF data or this needs a decoder.
                    Ok(bytes) => LcdContent::Gif {
                        frames: vec![bytes],
                        fps: 10,
                    },
                    Err(e) => {
                        tracing::warn!("Failed to read GIF file: {e}");
                        LcdContent::Off
                    }
                }
            } else {
                LcdContent::Off
            }
        }
        "Preset" => {
            if let Some(preset) = presets.iter().find(|p| p.name == file_path) {
                LcdContent::Preset(preset.clone())
            } else if !file_path.is_empty() {
                // Fallback: name typed manually, not in fetched list
                LcdContent::Preset(LcdPreset {
                    category: LcdPresetCategory::Cooler,
                    index: 0,
                    name: file_path.to_string(),
                    frame_count: 0,
                    fps: 24,
                    thumbnail: Vec::new(),
                })
            } else {
                LcdContent::Off
            }
        }
        _ => LcdContent::Off,
    };

    LcdConfig {
        brightness: frgb_model::Brightness::new(brightness.clamp(0, 255) as u8),
        rotation: rotation_from_degrees(rotation),
        content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::sensor::Sensor;

    #[test]
    fn rotation_from_degrees_all() {
        assert_eq!(rotation_from_degrees(0), LcdRotation::R0);
        assert_eq!(rotation_from_degrees(90), LcdRotation::R90);
        assert_eq!(rotation_from_degrees(180), LcdRotation::R180);
        assert_eq!(rotation_from_degrees(270), LcdRotation::R270);
        assert_eq!(rotation_from_degrees(999), LcdRotation::R0);
    }

    #[test]
    fn sensor_style_from_str_all() {
        assert_eq!(sensor_style_from_str("Gauge"), LcdSensorStyle::Gauge);
        assert_eq!(sensor_style_from_str("Number"), LcdSensorStyle::Number);
        assert_eq!(sensor_style_from_str("Graph"), LcdSensorStyle::Graph);
        assert_eq!(sensor_style_from_str("Carousel"), LcdSensorStyle::Carousel);
        assert_eq!(sensor_style_from_str("unknown"), LcdSensorStyle::Gauge);
    }

    #[test]
    fn sensor_color_from_str_all() {
        assert_eq!(sensor_color_from_str("Blue"), LcdSensorColor::Blue);
        assert_eq!(sensor_color_from_str("Green"), LcdSensorColor::Green);
        assert_eq!(sensor_color_from_str("Purple"), LcdSensorColor::Purple);
        assert_eq!(sensor_color_from_str("Red"), LcdSensorColor::Red);
        assert_eq!(sensor_color_from_str("unknown"), LcdSensorColor::Blue);
    }

    #[test]
    fn clock_style_from_index_all() {
        assert_eq!(clock_style_from_index(0), ClockStyle::C1);
        assert_eq!(clock_style_from_index(1), ClockStyle::C2);
        assert_eq!(clock_style_from_index(2), ClockStyle::C3);
        assert_eq!(clock_style_from_index(3), ClockStyle::C4);
        assert_eq!(clock_style_from_index(4), ClockStyle::C5);
        assert_eq!(clock_style_from_index(5), ClockStyle::C6);
        assert_eq!(clock_style_from_index(6), ClockStyle::C7);
        assert_eq!(clock_style_from_index(99), ClockStyle::C1);
    }

    #[test]
    fn build_lcd_config_off() {
        let cfg = build_lcd_config("Off", 128, 0, "", "", "", 0, "", "", &[]);
        assert_eq!(cfg.content, LcdContent::Off);
        assert_eq!(cfg.brightness, frgb_model::Brightness::new(128));
        assert_eq!(cfg.rotation, LcdRotation::R0);
    }

    #[test]
    fn build_lcd_config_sensor() {
        let cfg = build_lcd_config("Sensor", 200, 90, "CPU", "Gauge", "Blue", 0, "", "", &[]);
        match cfg.content {
            LcdContent::Sensor(ref disp) => {
                assert_eq!(disp.sensor, Sensor::Cpu);
                assert_eq!(disp.style, LcdSensorStyle::Gauge);
                assert_eq!(disp.color, LcdSensorColor::Blue);
                assert_eq!(disp.unit, TempUnit::Celsius);
                assert!(disp.label.is_none());
            }
            other => panic!("expected Sensor, got {:?}", other),
        }
        assert_eq!(cfg.rotation, LcdRotation::R90);
    }

    #[test]
    fn build_lcd_config_clock() {
        let cfg = build_lcd_config("Clock", 255, 180, "", "", "", 3, "", "", &[]);
        assert_eq!(cfg.content, LcdContent::Clock(ClockStyle::C4));
        assert_eq!(cfg.rotation, LcdRotation::R180);
    }

    #[test]
    fn build_lcd_config_system_info() {
        let cfg = build_lcd_config("System Info", 100, 270, "", "", "", 0, "", "", &[]);
        assert_eq!(cfg.content, LcdContent::SystemInfo);
        assert_eq!(cfg.rotation, LcdRotation::R270);
    }

    #[test]
    fn build_lcd_config_text() {
        let cfg = build_lcd_config("Text", 50, 0, "", "", "", 0, "Hello World", "", &[]);
        assert_eq!(cfg.content, LcdContent::Text("Hello World".into()));
    }

    #[test]
    fn build_lcd_config_brightness_clamped() {
        let cfg = build_lcd_config("Off", 300, 0, "", "", "", 0, "", "", &[]);
        assert_eq!(cfg.brightness, frgb_model::Brightness::new(255));

        let cfg2 = build_lcd_config("Off", -10, 0, "", "", "", 0, "", "", &[]);
        assert_eq!(cfg2.brightness, frgb_model::Brightness::new(0));
    }

    #[test]
    fn build_lcd_config_image_no_path() {
        let cfg = build_lcd_config("Image", 200, 0, "", "", "", 0, "", "", &[]);
        assert_eq!(cfg.content, LcdContent::Off);
    }

    #[test]
    fn build_lcd_config_gif_no_path() {
        let cfg = build_lcd_config("GIF", 200, 0, "", "", "", 0, "", "", &[]);
        assert_eq!(cfg.content, LcdContent::Off);
    }

    #[test]
    fn build_lcd_config_preset_no_name() {
        let cfg = build_lcd_config("Preset", 200, 0, "", "", "", 0, "", "", &[]);
        assert_eq!(cfg.content, LcdContent::Off);
    }

    #[test]
    fn build_lcd_config_preset_with_name_fallback() {
        // No presets list — falls back to hardcoded defaults
        let cfg = build_lcd_config("Preset", 200, 0, "", "", "", 0, "", "Snowfall", &[]);
        match cfg.content {
            LcdContent::Preset(ref p) => {
                assert_eq!(p.name, "Snowfall");
                assert_eq!(p.category, LcdPresetCategory::Cooler);
                assert_eq!(p.fps, 24);
                assert_eq!(p.index, 0);
            }
            other => panic!("expected Preset, got {:?}", other),
        }
    }

    #[test]
    fn build_lcd_config_preset_lookup_from_list() {
        let presets = vec![
            LcdPreset {
                category: LcdPresetCategory::Fan,
                index: 3,
                name: "Aurora".into(),
                frame_count: 120,
                fps: 30,
                thumbnail: Vec::new(),
            },
            LcdPreset {
                category: LcdPresetCategory::Cooler,
                index: 7,
                name: "Snowfall".into(),
                frame_count: 60,
                fps: 15,
                thumbnail: Vec::new(),
            },
        ];
        let cfg = build_lcd_config("Preset", 200, 0, "", "", "", 0, "", "Snowfall", &presets);
        match cfg.content {
            LcdContent::Preset(ref p) => {
                assert_eq!(p.name, "Snowfall");
                assert_eq!(p.category, LcdPresetCategory::Cooler);
                assert_eq!(p.index, 7);
                assert_eq!(p.frame_count, 60);
                assert_eq!(p.fps, 15);
            }
            other => panic!("expected Preset, got {:?}", other),
        }
    }
}
