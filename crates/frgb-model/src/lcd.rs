use crate::sensor::{Sensor, TempUnit};
use crate::Brightness;
use crate::ValidatedName;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LcdContent {
    Off,
    Image(Vec<u8>),
    Text(String),
    Sensor(LcdSensorDisplay),
    SensorCarousel(Vec<LcdSensorDisplay>),
    SystemInfo,
    Clock(ClockStyle),
    Gif {
        frames: Vec<Vec<u8>>,
        fps: u8,
    },
    Video(Vec<u8>),
    Preset(LcdPreset),
    Template(LcdTemplate),
    /// Live screen capture streamed to LCD.
    ScreenCapture {
        display: Option<String>,
        window: Option<String>,
        fps: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcdSensorDisplay {
    pub sensor: Sensor,
    pub label: Option<String>,
    pub unit: TempUnit,
    pub style: LcdSensorStyle,
    pub color: LcdSensorColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LcdSensorStyle {
    Gauge,
    Number,
    Graph,
    Carousel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LcdSensorColor {
    Blue,
    Green,
    Purple,
    Red,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClockStyle {
    C1,
    C2,
    C3,
    C4,
    C5,
    C6,
    C7,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcdPreset {
    pub category: LcdPresetCategory,
    pub index: u8,
    pub name: String,
    pub frame_count: u16,
    pub fps: u8,
    /// First frame JPEG bytes for thumbnail preview (populated by ListPresets).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thumbnail: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LcdPresetCategory {
    Cooler,
    Fan,
    Led,
    Ga2v,
    Legacy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcdConfig {
    pub brightness: Brightness,
    pub rotation: LcdRotation,
    pub content: LcdContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LcdRotation {
    R0,
    R90,
    R180,
    R270,
}

/// Info about a single LCD screen, returned by ListLcdDevices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcdDeviceInfo {
    /// Index in the LCD backend (stable within a session).
    pub index: u8,
    /// Display name (e.g. "SL-LCD Wireless 1", "HydroShift II Circle").
    pub name: String,
    /// Display resolution.
    pub width: u32,
    pub height: u32,
}

// ---------------------------------------------------------------------------
// LCD Template System — composable widget-based display
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcdTemplate {
    pub id: String,
    pub name: ValidatedName,
    pub base_width: u32,
    pub base_height: u32,
    pub background: TemplateBackground,
    pub widgets: Vec<Widget>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TemplateBackground {
    Color { rgba: [u8; 4] },
    Image { path: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Widget {
    pub id: String,
    pub kind: WidgetKind,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub rotation: f32,
    #[serde(default = "default_true")]
    pub visible: bool,
    pub update_interval_ms: Option<u64>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WidgetKind {
    Label {
        text: String,
        font_size: f32,
        color: [u8; 4],
        #[serde(default)]
        align: TextAlign,
    },
    ValueText {
        source: SensorSourceConfig,
        #[serde(default = "default_format")]
        format: String,
        #[serde(default)]
        unit: String,
        font_size: f32,
        color: [u8; 4],
        #[serde(default)]
        align: TextAlign,
        #[serde(default)]
        value_min: f32,
        #[serde(default = "default_value_max")]
        value_max: f32,
        #[serde(default)]
        ranges: Vec<SensorRange>,
    },
    RadialGauge {
        source: SensorSourceConfig,
        #[serde(default)]
        value_min: f32,
        #[serde(default = "default_value_max")]
        value_max: f32,
        #[serde(default = "default_start_angle")]
        start_angle: f32,
        #[serde(default = "default_sweep_angle")]
        sweep_angle: f32,
        #[serde(default = "default_inner_radius")]
        inner_radius_pct: f32,
        #[serde(default)]
        background_color: [u8; 4],
        #[serde(default)]
        ranges: Vec<SensorRange>,
    },
    VerticalBar {
        source: SensorSourceConfig,
        #[serde(default)]
        value_min: f32,
        #[serde(default = "default_value_max")]
        value_max: f32,
        #[serde(default)]
        background_color: [u8; 4],
        #[serde(default)]
        corner_radius: f32,
        #[serde(default)]
        ranges: Vec<SensorRange>,
    },
    HorizontalBar {
        source: SensorSourceConfig,
        #[serde(default)]
        value_min: f32,
        #[serde(default = "default_value_max")]
        value_max: f32,
        #[serde(default)]
        background_color: [u8; 4],
        #[serde(default)]
        corner_radius: f32,
        #[serde(default)]
        ranges: Vec<SensorRange>,
    },
    Speedometer {
        source: SensorSourceConfig,
        #[serde(default)]
        value_min: f32,
        #[serde(default = "default_value_max")]
        value_max: f32,
        #[serde(default = "default_start_angle")]
        start_angle: f32,
        #[serde(default = "default_sweep_angle")]
        sweep_angle: f32,
        needle_color: [u8; 4],
        tick_color: [u8; 4],
        #[serde(default = "default_tick_count")]
        tick_count: u32,
        #[serde(default)]
        background_color: [u8; 4],
        #[serde(default)]
        ranges: Vec<SensorRange>,
    },
    CoreBars {
        #[serde(default)]
        sources: Vec<SensorSourceConfig>,
        #[serde(default)]
        orientation: BarOrientation,
        #[serde(default)]
        background_color: [u8; 4],
        #[serde(default = "default_true")]
        show_labels: bool,
        #[serde(default)]
        ranges: Vec<SensorRange>,
    },
    Image {
        path: String,
        #[serde(default = "default_opacity")]
        opacity: f32,
    },
}

fn default_format() -> String {
    "{:.0}".into()
}
fn default_value_max() -> f32 {
    100.0
}
fn default_start_angle() -> f32 {
    135.0
}
fn default_sweep_angle() -> f32 {
    270.0
}
fn default_inner_radius() -> f32 {
    0.78
}
fn default_tick_count() -> u32 {
    10
}
fn default_opacity() -> f32 {
    1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BarOrientation {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensorRange {
    pub max: Option<f32>,
    pub color: [u8; 3],
    #[serde(default = "default_alpha")]
    pub alpha: u8,
}

fn default_alpha() -> u8 {
    255
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SensorSourceConfig {
    CpuTemp,
    GpuTemp,
    GpuUsage,
    WaterTemp,
    CpuUsage,
    MemUsage,
    Hwmon {
        name: String,
        label: String,
    },
    Constant {
        value: f32,
    },
    /// Run a shell command and parse stdout as f32.
    Command {
        cmd: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor::{Sensor, TempUnit};
    use crate::Brightness;

    fn base_config(content: LcdContent) -> LcdConfig {
        LcdConfig {
            brightness: Brightness::new(200),
            rotation: LcdRotation::R0,
            content,
        }
    }

    #[test]
    fn lcd_config_off_roundtrip() {
        let cfg = base_config(LcdContent::Off);
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: LcdConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deser);
    }

    #[test]
    fn lcd_config_text_roundtrip() {
        let cfg = base_config(LcdContent::Text("CPU: 55°C".into()));
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: LcdConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deser);
    }

    #[test]
    fn lcd_config_clock_roundtrip() {
        let cfg = base_config(LcdContent::Clock(ClockStyle::C3));
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: LcdConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deser);
    }

    #[test]
    fn lcd_config_gif_roundtrip() {
        let cfg = base_config(LcdContent::Gif {
            frames: vec![vec![0u8; 4], vec![1u8; 4]],
            fps: 15,
        });
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: LcdConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deser);
    }

    #[test]
    fn lcd_config_sensor_carousel_roundtrip() {
        let displays = vec![
            LcdSensorDisplay {
                sensor: Sensor::Cpu,
                label: Some("CPU Temp".into()),
                unit: TempUnit::Celsius,
                style: LcdSensorStyle::Gauge,
                color: LcdSensorColor::Blue,
            },
            LcdSensorDisplay {
                sensor: Sensor::Gpu,
                label: None,
                unit: TempUnit::Fahrenheit,
                style: LcdSensorStyle::Number,
                color: LcdSensorColor::Red,
            },
        ];
        let cfg = base_config(LcdContent::SensorCarousel(displays));
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: LcdConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deser);
    }

    #[test]
    fn lcd_config_preset_roundtrip() {
        let cfg = base_config(LcdContent::Preset(LcdPreset {
            category: LcdPresetCategory::Cooler,
            index: 3,
            name: "Snowfall".into(),
            frame_count: 60,
            fps: 30,
            thumbnail: Vec::new(),
        }));
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: LcdConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deser);
    }

    #[test]
    fn lcd_config_system_info_roundtrip() {
        let cfg = base_config(LcdContent::SystemInfo);
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: LcdConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deser);
    }

    #[test]
    fn lcd_rotation_all_variants_roundtrip() {
        for rot in [LcdRotation::R0, LcdRotation::R90, LcdRotation::R180, LcdRotation::R270] {
            let json = serde_json::to_string(&rot).unwrap();
            let deser: LcdRotation = serde_json::from_str(&json).unwrap();
            assert_eq!(rot, deser);
        }
    }

    #[test]
    fn lcd_template_roundtrip() {
        let template = LcdTemplate {
            id: "test-1".into(),
            name: ValidatedName::new("My Dashboard").unwrap(),
            base_width: 480,
            base_height: 480,
            background: TemplateBackground::Color { rgba: [0, 0, 0, 255] },
            widgets: vec![Widget {
                id: "gauge-1".into(),
                kind: WidgetKind::RadialGauge {
                    source: SensorSourceConfig::CpuTemp,
                    value_min: 20.0,
                    value_max: 100.0,
                    start_angle: 135.0,
                    sweep_angle: 270.0,
                    inner_radius_pct: 0.78,
                    background_color: [40, 40, 40, 255],
                    ranges: vec![
                        SensorRange {
                            max: Some(60.0),
                            color: [0, 200, 100],
                            alpha: 255,
                        },
                        SensorRange {
                            max: None,
                            color: [255, 50, 50],
                            alpha: 255,
                        },
                    ],
                },
                x: 240.0,
                y: 240.0,
                width: 300.0,
                height: 300.0,
                rotation: 0.0,
                visible: true,
                update_interval_ms: Some(1000),
            }],
        };
        let json = serde_json::to_string(&template).unwrap();
        let deser: LcdTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(template, deser);
    }

    #[test]
    fn lcd_content_template_roundtrip() {
        let cfg = base_config(LcdContent::Template(LcdTemplate {
            id: "t1".into(),
            name: ValidatedName::new("Test").unwrap(),
            base_width: 400,
            base_height: 400,
            background: TemplateBackground::Color { rgba: [0, 0, 0, 255] },
            widgets: vec![],
        }));
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: LcdConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deser);
    }

    #[test]
    fn widget_defaults_from_minimal_json() {
        let json = r#"{"id":"w","kind":{"Label":{"text":"Hi","font_size":24.0,"color":[255,255,255,255]}},"x":0.0,"y":0.0,"width":100.0,"height":30.0}"#;
        let widget: Widget = serde_json::from_str(json).unwrap();
        assert!(widget.visible);
        assert_eq!(widget.rotation, 0.0);
        assert!(widget.update_interval_ms.is_none());
    }

    #[test]
    fn command_sensor_source_roundtrip() {
        let src = SensorSourceConfig::Command {
            cmd: "echo 42.5".into(),
        };
        let json = serde_json::to_string(&src).unwrap();
        let deser: SensorSourceConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(deser, SensorSourceConfig::Command { cmd } if cmd == "echo 42.5"));
    }
}
