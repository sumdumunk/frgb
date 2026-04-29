//! Converts frgb-model types to Slint-generated UI types.

use slint::{ModelRc, SharedString, VecModel};

use frgb_model::config::GroupStatus;
use frgb_model::device::DeviceType;
use frgb_model::rgb::RgbMode;
use frgb_model::sensor::SensorInfo;
use frgb_model::speed::SpeedMode;
use frgb_rgb::layout::LedLayout;

use crate::{DeviceGroupData, SensorData};

// ---------------------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------------------

pub fn device_type_display(dt: &DeviceType) -> &'static str {
    match dt {
        DeviceType::SlWireless => "SL Wireless",
        DeviceType::SlLcdWireless => "SL LCD Wireless",
        DeviceType::SlInfWireless => "SL INF Wireless",
        DeviceType::ClWireless => "CL Wireless",
        DeviceType::TlWireless => "TL Wireless",
        DeviceType::TlLcdWireless => "TL LCD Wireless",
        DeviceType::SlV2 => "SL V2",
        DeviceType::P28 => "P28",
        DeviceType::Rl120 => "RL120",
        DeviceType::HydroShift => "HydroShift",
        DeviceType::HydroShiftII => "HydroShift II",
        DeviceType::GalahadIiLcd => "Galahad II LCD",
        DeviceType::GalahadIiTrinity => "Galahad II Trinity",
        DeviceType::V150 => "V150",
        DeviceType::StrimerWireless => "Strimer Wireless",
        DeviceType::StrimerPlusV2 => "Strimer Plus V2",
        DeviceType::SideArgbKit => "Side ARGB Kit",
        DeviceType::WaterBlock => "Water Block",
        DeviceType::WaterBlock2 => "Water Block 2",
        DeviceType::Led88 => "LED 88",
        DeviceType::Ga2 => "GA II",
        DeviceType::Lc217 => "LC217",
        DeviceType::OpenRgb => "OpenRGB",
        DeviceType::Aura => "Aura",
        DeviceType::Unknown => "Unknown",
    }
}

pub fn device_type_from_display(s: &str) -> DeviceType {
    match s {
        "SL Wireless" => DeviceType::SlWireless,
        "SL LCD Wireless" => DeviceType::SlLcdWireless,
        "SL INF Wireless" => DeviceType::SlInfWireless,
        "CL Wireless" => DeviceType::ClWireless,
        "TL Wireless" => DeviceType::TlWireless,
        "TL LCD Wireless" => DeviceType::TlLcdWireless,
        "SL V2" => DeviceType::SlV2,
        "P28" => DeviceType::P28,
        "RL120" => DeviceType::Rl120,
        "HydroShift" => DeviceType::HydroShift,
        "HydroShift II" => DeviceType::HydroShiftII,
        "Galahad II LCD" => DeviceType::GalahadIiLcd,
        "Galahad II Trinity" => DeviceType::GalahadIiTrinity,
        "V150" => DeviceType::V150,
        "Strimer Wireless" => DeviceType::StrimerWireless,
        "Strimer Plus V2" => DeviceType::StrimerPlusV2,
        "Side ARGB Kit" => DeviceType::SideArgbKit,
        "Water Block" => DeviceType::WaterBlock,
        "Water Block 2" => DeviceType::WaterBlock2,
        "LED 88" => DeviceType::Led88,
        "GA II" => DeviceType::Ga2,
        "LC217" => DeviceType::Lc217,
        "OpenRGB" => DeviceType::OpenRgb,
        "Aura" => DeviceType::Aura,
        _ => DeviceType::Unknown,
    }
}

pub fn role_string(role: &frgb_model::device::FanRole) -> String {
    match role {
        frgb_model::device::FanRole::Intake => "Intake".into(),
        frgb_model::device::FanRole::Exhaust => "Exhaust".into(),
        frgb_model::device::FanRole::Pump => "Pump".into(),
        frgb_model::device::FanRole::Custom(s) => s.clone(),
    }
}

pub fn speed_mode_string(mode: &SpeedMode) -> &'static str {
    match mode {
        SpeedMode::Manual(_) => "Manual",
        SpeedMode::Pwm => "PWM",
        SpeedMode::Curve(_) => "Curve",
        SpeedMode::NamedCurve(_) => "Curve",
    }
}

/// Extract the manual speed percent, or -1 for non-manual modes.
pub fn speed_percent(mode: &SpeedMode) -> i32 {
    match mode {
        SpeedMode::Manual(pct) => pct.value() as i32,
        _ => -1,
    }
}

pub fn speed_curve_name(mode: &SpeedMode) -> &str {
    match mode {
        SpeedMode::NamedCurve(name) => name,
        SpeedMode::Curve(_) => "(custom)",
        _ => "",
    }
}

pub fn rgb_mode_display(mode: &RgbMode) -> &'static str {
    match mode {
        RgbMode::Off => "Off",
        RgbMode::Static { .. } => "Static",
        RgbMode::PerFan(_) => "Per-Fan",
        RgbMode::PerLed(_) => "Per-LED",
        RgbMode::Effect { .. } => "Effect",
        RgbMode::TempRgb(_) => "Temp RGB",
        RgbMode::Composed(_) => "Composed",
        RgbMode::SubZones { .. } => "Sub-Zones",
    }
}

// ---------------------------------------------------------------------------
// Model -> Slint conversions
// ---------------------------------------------------------------------------

/// Average of non-zero RPM values. Returns 0 if all are zero or empty.
pub fn avg_nonzero_rpm(rpms: &[i32]) -> i32 {
    let (sum, count) = rpms
        .iter()
        .filter(|&&r| r > 0)
        .fold((0i32, 0i32), |(s, c), &r| (s + r, c + 1));
    if count == 0 {
        0
    } else {
        sum / count
    }
}

pub fn group_status_to_slint(gs: &GroupStatus) -> DeviceGroupData {
    let g = &gs.group;
    let rpms_i32: Vec<i32> = gs.rpms.iter().map(|&r| r as i32).collect();
    let avg_rpm = avg_nonzero_rpm(&rpms_i32);

    DeviceGroupData {
        group_id: g.id.value() as i32,
        name: SharedString::from(&g.name),
        device_type: SharedString::from(device_type_display(&g.device_type)),
        fan_count: g.fan_count as i32,
        role: SharedString::from(role_string(&g.role)),
        online: gs.online,
        rpms: ModelRc::new(VecModel::from(rpms_i32)),
        avg_rpm,
        speed_percent: speed_percent(&gs.speed),
        speed_mode: SharedString::from(speed_mode_string(&gs.speed)),
        speed_curve_name: SharedString::from(speed_curve_name(&gs.speed)),
        rgb_mode: SharedString::from(rgb_mode_display(&gs.rgb)),
        mb_sync: gs.mb_sync,
        has_lcd: gs.lcd.is_some(),
        lcd_count: gs.lcd_count as i32,
        inner_leds: LedLayout::for_device(g.device_type).inner_count as i32,
        outer_leds: LedLayout::for_device(g.device_type).outer_count as i32,
        cfm: {
            let pct = speed_percent(&gs.speed) as f32 / 100.0;
            let per_fan = g.cfm_max.unwrap_or(0.0);
            per_fan * g.fan_count as f32 * pct
        },
    }
}

#[allow(dead_code)] // Will be used when sensor page is migrated
pub fn sensor_info_to_slint(si: &SensorInfo) -> SensorData {
    SensorData {
        name: SharedString::from(si.name.as_str()),
        label: SharedString::from(sensor_label(&si.sensor)),
        value: si.current,
        available: si.available,
    }
}

/// Returns a display label for a sensor.
pub fn sensor_label(sensor: &frgb_model::sensor::Sensor) -> String {
    match sensor {
        frgb_model::sensor::Sensor::Cpu => "CPU".into(),
        frgb_model::sensor::Sensor::Gpu => "GPU".into(),
        frgb_model::sensor::Sensor::GpuHotspot => "GPU Hotspot".into(),
        frgb_model::sensor::Sensor::GpuVram => "GPU VRAM".into(),
        frgb_model::sensor::Sensor::GpuPower => "GPU Power".into(),
        frgb_model::sensor::Sensor::GpuUsage => "GPU Usage".into(),
        frgb_model::sensor::Sensor::Water => "Water".into(),
        frgb_model::sensor::Sensor::Motherboard { channel } => format!("Motherboard {channel}"),
        frgb_model::sensor::Sensor::Weighted { cpu_pct, gpu_pct } => {
            format!("Weighted {cpu_pct}/{gpu_pct}")
        }
    }
}

/// Inverse of `sensor_label` — parses labels like "Motherboard 3", "Weighted 60/40".
pub fn sensor_from_label(label: &str) -> frgb_model::sensor::Sensor {
    use frgb_model::sensor::Sensor;
    if let Some(ch) = label.strip_prefix("Motherboard ") {
        let channel = ch.parse::<u8>().unwrap_or(0);
        return Sensor::Motherboard { channel };
    }
    if label == "Motherboard" {
        return Sensor::Motherboard { channel: 0 };
    }
    if let Some(pcts) = label.strip_prefix("Weighted ") {
        if let Some((cpu, gpu)) = pcts.split_once('/') {
            let cpu_pct = cpu.parse::<u8>().unwrap_or(50);
            let gpu_pct = gpu.parse::<u8>().unwrap_or(50);
            return Sensor::Weighted { cpu_pct, gpu_pct };
        }
    }
    match label {
        "CPU" => Sensor::Cpu,
        "GPU" => Sensor::Gpu,
        "GPU Hotspot" => Sensor::GpuHotspot,
        "GPU VRAM" => Sensor::GpuVram,
        "GPU Power" => Sensor::GpuPower,
        "GPU Usage" => Sensor::GpuUsage,
        "Water" => Sensor::Water,
        _ => {
            tracing::warn!("unrecognized sensor label '{label}', defaulting to CPU");
            Sensor::Cpu
        }
    }
}
