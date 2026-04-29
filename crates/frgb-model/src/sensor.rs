use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Sensor {
    Cpu,
    Gpu,
    /// GPU hotspot / junction temperature.
    GpuHotspot,
    /// VRAM temperature.
    GpuVram,
    /// GPU power draw (watts).
    GpuPower,
    /// GPU utilization percentage (0–100).
    GpuUsage,
    Water,
    Motherboard {
        channel: u8,
    },
    Weighted {
        cpu_pct: u8,
        gpu_pct: u8,
    }, // percentages 0-100
}

impl Sensor {
    pub fn validate(&self) -> Result<(), String> {
        if let Sensor::Weighted { cpu_pct, gpu_pct } = self {
            if (*cpu_pct as u16 + *gpu_pct as u16) != 100 {
                return Err(format!(
                    "weighted percentages must sum to 100, got {}+{}",
                    cpu_pct, gpu_pct
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensorInfo {
    pub sensor: Sensor,
    pub name: String,
    pub current: f32,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensorCalibration {
    pub cpu_offset: f32,
    pub gpu_offset: f32,
    pub custom_paths: HashMap<String, String>,
}

impl Default for SensorCalibration {
    fn default() -> Self {
        Self {
            cpu_offset: 0.0,
            gpu_offset: 0.0,
            custom_paths: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TempUnit {
    Celsius,
    Fahrenheit,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn sensor_calibration_default_values() {
        let cal = SensorCalibration::default();
        assert_eq!(cal.cpu_offset, 0.0);
        assert_eq!(cal.gpu_offset, 0.0);
        assert!(cal.custom_paths.is_empty());
    }

    #[test]
    fn sensor_eq_and_hash_cpu() {
        let a = Sensor::Cpu;
        let b = Sensor::Cpu;
        assert_eq!(a, b);
        let mut map: HashMap<Sensor, u32> = HashMap::new();
        map.insert(a, 42);
        assert_eq!(map[&b], 42);
    }

    #[test]
    fn sensor_eq_and_hash_gpu() {
        let a = Sensor::Gpu;
        let b = Sensor::Gpu;
        assert_eq!(a, b);
        let mut map: HashMap<Sensor, &str> = HashMap::new();
        map.insert(a, "gpu");
        assert_eq!(map[&b], "gpu");
    }

    #[test]
    fn sensor_eq_and_hash_motherboard() {
        let a = Sensor::Motherboard { channel: 3 };
        let b = Sensor::Motherboard { channel: 3 };
        let c = Sensor::Motherboard { channel: 5 };
        assert_eq!(a, b);
        assert_ne!(a, c);
        let mut map: HashMap<Sensor, u8> = HashMap::new();
        map.insert(a, 3);
        assert_eq!(map[&b], 3);
        assert!(!map.contains_key(&c));
    }

    #[test]
    fn sensor_eq_and_hash_weighted() {
        let a = Sensor::Weighted {
            cpu_pct: 70,
            gpu_pct: 30,
        };
        let b = Sensor::Weighted {
            cpu_pct: 70,
            gpu_pct: 30,
        };
        let c = Sensor::Weighted {
            cpu_pct: 50,
            gpu_pct: 50,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
        let mut map: HashMap<Sensor, bool> = HashMap::new();
        map.insert(a, true);
        assert!(map[&b]);
        assert!(!map.contains_key(&c));
    }

    #[test]
    fn sensor_variants_are_not_equal_across_types() {
        assert_ne!(Sensor::Cpu, Sensor::Gpu);
        assert_ne!(Sensor::Cpu, Sensor::Water);
        assert_ne!(Sensor::Gpu, Sensor::Water);
        assert_ne!(Sensor::Cpu, Sensor::Motherboard { channel: 0 });
    }

    #[test]
    fn sensor_calibration_custom_paths() {
        let mut cal = SensorCalibration::default();
        cal.custom_paths
            .insert("cpu_die".into(), "/sys/class/thermal/thermal_zone0/temp".into());
        assert_eq!(cal.custom_paths.len(), 1);
        assert!(cal.custom_paths.contains_key("cpu_die"));
    }

    #[test]
    fn sensor_calibration_serialization_roundtrip() {
        let mut cal = SensorCalibration {
            cpu_offset: 5.0,
            gpu_offset: -2.5,
            ..Default::default()
        };
        cal.custom_paths.insert("my_sensor".into(), "/path/to/file".into());
        let json = serde_json::to_string(&cal).expect("serialize");
        let restored: SensorCalibration = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(cal, restored);
    }

    #[test]
    fn sensor_weighted_validate_ok() {
        let s = Sensor::Weighted {
            cpu_pct: 70,
            gpu_pct: 30,
        };
        assert!(s.validate().is_ok());
    }

    #[test]
    fn sensor_weighted_validate_not_100() {
        let s = Sensor::Weighted {
            cpu_pct: 60,
            gpu_pct: 30,
        };
        let err = s.validate().unwrap_err();
        assert!(err.contains("must sum to 100"));
    }

    #[test]
    fn sensor_non_weighted_validate_ok() {
        assert!(Sensor::Cpu.validate().is_ok());
        assert!(Sensor::Gpu.validate().is_ok());
        assert!(Sensor::Water.validate().is_ok());
        assert!(Sensor::Motherboard { channel: 2 }.validate().is_ok());
    }

    #[test]
    fn sensor_water_serialization_roundtrip() {
        let s = Sensor::Water;
        let json = serde_json::to_string(&s).expect("serialize");
        let restored: Sensor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(s, restored);
    }
}
