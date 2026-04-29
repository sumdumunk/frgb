use serde::{Deserialize, Serialize};

use crate::sensor::Sensor;
use crate::GroupId;
use crate::SpeedPercent;
use crate::Temperature;

/// A single point on a fan curve: at `temp` degrees, target `speed` percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurvePoint {
    /// Temperature in degrees Celsius.
    pub temp: Temperature,
    /// Fan speed as a percentage (0–100).
    pub speed: SpeedPercent,
}

/// How values between curve points are computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interpolation {
    /// Linearly interpolate between surrounding points.
    Linear,
    /// Hold the lower point's speed until the next point is reached.
    Step,
}

/// Optional stop-below configuration: fan stops completely when temperature
/// drops below `temp`, and restarts at `start_speed` when it rises again.
/// `hysteresis` is the number of degrees below `temp` the temperature must
/// fall before the fan stops again after it has restarted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopBelow {
    pub temp: Temperature,
    pub start_speed: SpeedPercent,
    pub hysteresis: u8,
}

/// A temperature-driven fan curve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanCurve {
    /// Control points, must have ascending `temp` values.
    pub points: Vec<CurvePoint>,
    /// Which temperature sensor drives this curve.
    pub sensor: Sensor,
    pub interpolation: Interpolation,
    /// Minimum speed (%) — output is never below this value.
    pub min_speed: SpeedPercent,
    /// If set, the fan may stop entirely below the given temperature.
    pub stop_below: Option<StopBelow>,
    /// Maximum speed change per update cycle (% per second). `None` = instant.
    pub ramp_rate: Option<u8>,
}

impl FanCurve {
    /// Validate that curve points have strictly ascending temperatures and
    /// that there is at least one point.
    pub fn validate(&self) -> Result<(), String> {
        if self.points.is_empty() {
            return Err("FanCurve must have at least one point".into());
        }
        for window in self.points.windows(2) {
            if window[1].temp <= window[0].temp {
                return Err(format!(
                    "FanCurve points must have strictly ascending temps: {} is not > {}",
                    window[1].temp, window[0].temp
                ));
            }
        }
        Ok(())
    }

    /// Return the interpolated fan speed (%) for a given temperature.
    ///
    /// - If `stop_below` is set and `temp` is below the threshold, returns 0 (fan stopped).
    /// - Below first point → first point's speed
    /// - Above last point  → last point's speed
    /// - Linear: interpolate between surrounding points (rounded to nearest integer)
    /// - Step: use the lower point's speed until the upper point's temp is reached
    /// - The result is always at least `min_speed`.
    pub fn speed_at_temp(&self, temp: Temperature) -> u8 {
        let min = self.min_speed.value();

        // Empty curve: return min_speed as safe fallback (never panic).
        if self.points.is_empty() {
            return min;
        }

        // Stop-below: fan off when temp is below the configured threshold.
        if let Some(ref sb) = self.stop_below {
            if temp < sb.temp {
                return 0;
            }
        }

        let points = &self.points;

        // Edge cases: outside range
        if temp <= points[0].temp {
            return points[0].speed.value().max(min);
        }
        if temp >= points[points.len() - 1].temp {
            return points[points.len() - 1].speed.value().max(min);
        }

        // Find surrounding points
        let upper_idx = points.partition_point(|p| p.temp <= temp);
        let lower = &points[upper_idx - 1];
        let upper = &points[upper_idx];

        let speed = match self.interpolation {
            Interpolation::Step => lower.speed.value(),
            Interpolation::Linear => {
                let t_range = (upper.temp.celsius() - lower.temp.celsius()) as f64;
                let t_pos = (temp.celsius() - lower.temp.celsius()) as f64;
                let s_range = upper.speed.value() as f64 - lower.speed.value() as f64;
                let speed = lower.speed.value() as f64 + s_range * (t_pos / t_range);
                speed.round().clamp(0.0, 255.0) as u8
            }
        };
        speed.max(min)
    }
}

/// Analysis result comparing a fan curve against observed thermal headroom.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurveSuggestion {
    /// Which fan group this applies to.
    pub group: GroupId,
    /// Human-readable curve name ("inline" if not saved).
    pub curve_name: String,
    /// The sensor driving this curve.
    pub sensor: Sensor,
    /// Human-readable suggestion message.
    pub message: String,
    /// Max observed temperature for this sensor over the tracking window (°C).
    pub observed_max_temp: f32,
    /// Temperature at which the curve reaches max speed (°C).
    pub curve_max_speed_temp: i32,
}

/// Pump operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PumpMode {
    /// RpmMode 0 — low noise
    Quiet,
    /// RpmMode 1 — standard cooling
    Standard,
    /// RpmMode 2 — increased airflow
    High,
    /// RpmMode 3 — maximum cooling
    Full,
    /// RpmMode 8 — fixed PWM duty cycle (0–100).
    Fixed(u8),
}

/// How a fan or pump channel determines its speed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpeedMode {
    /// Fixed percentage (0–100).
    Manual(SpeedPercent),
    /// Motherboard PWM control (no parameter — let the motherboard handle it).
    Pwm,
    /// Temperature-driven fan curve (inline definition).
    Curve(FanCurve),
    /// Reference to a named fan curve stored in config.
    NamedCurve(String),
}

impl SpeedMode {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            SpeedMode::Manual(_) => Ok(()), // SpeedPercent clamps on construction
            SpeedMode::Curve(curve) => curve.validate(),
            _ => Ok(()),
        }
    }
}

impl PumpMode {
    pub fn validate(&self) -> Result<(), String> {
        if let PumpMode::Fixed(pct) = self {
            if *pct > 100 {
                return Err(format!("fixed pump speed must be 0-100, got {pct}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor::Sensor;
    use crate::SpeedPercent;
    use crate::Temperature;

    fn sp(v: u8) -> SpeedPercent {
        SpeedPercent::new(v)
    }

    fn t(c: i32) -> Temperature {
        Temperature::new(c)
    }

    #[test]
    fn curve_point_ordering() {
        let a = CurvePoint {
            temp: t(30),
            speed: sp(25),
        };
        let b = CurvePoint {
            temp: t(50),
            speed: sp(50),
        };
        assert!(a.temp < b.temp);
    }

    #[test]
    fn fan_curve_validates_ascending_temps() {
        let curve = FanCurve {
            points: vec![
                CurvePoint {
                    temp: t(30),
                    speed: sp(25),
                },
                CurvePoint {
                    temp: t(50),
                    speed: sp(50),
                },
                CurvePoint {
                    temp: t(70),
                    speed: sp(80),
                },
            ],
            sensor: Sensor::Cpu,
            interpolation: Interpolation::Linear,
            min_speed: sp(25),
            stop_below: None,
            ramp_rate: None,
        };
        assert!(curve.validate().is_ok());
    }

    #[test]
    fn fan_curve_rejects_non_ascending_temps() {
        let curve = FanCurve {
            points: vec![
                CurvePoint {
                    temp: t(50),
                    speed: sp(50),
                },
                CurvePoint {
                    temp: t(30),
                    speed: sp(25),
                },
            ],
            sensor: Sensor::Cpu,
            interpolation: Interpolation::Linear,
            min_speed: sp(25),
            stop_below: None,
            ramp_rate: None,
        };
        assert!(curve.validate().is_err());
    }

    #[test]
    fn fan_curve_interpolate_linear() {
        let curve = FanCurve {
            points: vec![
                CurvePoint {
                    temp: t(30),
                    speed: sp(25),
                },
                CurvePoint {
                    temp: t(70),
                    speed: sp(100),
                },
            ],
            sensor: Sensor::Cpu,
            interpolation: Interpolation::Linear,
            min_speed: sp(25),
            stop_below: None,
            ramp_rate: None,
        };
        assert_eq!(curve.speed_at_temp(t(30)), 25);
        assert_eq!(curve.speed_at_temp(t(50)), 63); // 62.5 rounds to 63
        assert_eq!(curve.speed_at_temp(t(70)), 100);
        assert_eq!(curve.speed_at_temp(t(20)), 25); // below range
        assert_eq!(curve.speed_at_temp(t(80)), 100); // above range
    }

    #[test]
    fn fan_curve_interpolate_step() {
        let curve = FanCurve {
            points: vec![
                CurvePoint {
                    temp: t(30),
                    speed: sp(25),
                },
                CurvePoint {
                    temp: t(50),
                    speed: sp(50),
                },
                CurvePoint {
                    temp: t(70),
                    speed: sp(100),
                },
            ],
            sensor: Sensor::Cpu,
            interpolation: Interpolation::Step,
            min_speed: sp(25),
            stop_below: None,
            ramp_rate: None,
        };
        assert_eq!(curve.speed_at_temp(t(40)), 25);
        assert_eq!(curve.speed_at_temp(t(50)), 50);
        assert_eq!(curve.speed_at_temp(t(60)), 50);
    }

    #[test]
    fn speed_mode_validate_manual_ok() {
        assert!(SpeedMode::Manual(sp(100)).validate().is_ok());
        assert!(SpeedMode::Manual(sp(0)).validate().is_ok());
        assert!(SpeedMode::Pwm.validate().is_ok());
        assert!(SpeedMode::NamedCurve("balanced".into()).validate().is_ok());
    }

    #[test]
    fn speed_mode_validate_manual_clamped() {
        // SpeedPercent::new(101) clamps to 100 — no error possible from Manual
        assert_eq!(SpeedPercent::new(101).value(), 100);
        assert!(SpeedMode::Manual(sp(100)).validate().is_ok());
    }

    #[test]
    fn speed_mode_validate_curve_delegates() {
        let bad_curve = FanCurve {
            points: vec![],
            sensor: Sensor::Cpu,
            interpolation: Interpolation::Linear,
            min_speed: sp(0),
            stop_below: None,
            ramp_rate: None,
        };
        assert!(SpeedMode::Curve(bad_curve).validate().is_err());
    }

    #[test]
    fn pump_mode_validate_ok() {
        assert!(PumpMode::Quiet.validate().is_ok());
        assert!(PumpMode::Standard.validate().is_ok());
        assert!(PumpMode::High.validate().is_ok());
        assert!(PumpMode::Full.validate().is_ok());
        assert!(PumpMode::Fixed(100).validate().is_ok());
        assert!(PumpMode::Fixed(0).validate().is_ok());
    }

    #[test]
    fn pump_mode_validate_fixed_over_100() {
        let err = PumpMode::Fixed(101).validate().unwrap_err();
        assert!(err.contains("fixed pump speed must be 0-100"));
    }

    #[test]
    fn speed_mode_serialization() {
        let mode = SpeedMode::Manual(sp(50));
        let json = serde_json::to_string(&mode).unwrap();
        let deser: SpeedMode = serde_json::from_str(&json).unwrap();
        assert_eq!(deser, SpeedMode::Manual(sp(50)));
    }

    #[test]
    fn validate_empty_curve() {
        let curve = FanCurve {
            points: vec![],
            sensor: Sensor::Cpu,
            interpolation: Interpolation::Linear,
            min_speed: sp(25),
            stop_below: None,
            ramp_rate: None,
        };
        assert!(curve.validate().is_err());
    }

    #[test]
    fn validate_single_point_ok() {
        let curve = FanCurve {
            points: vec![CurvePoint {
                temp: t(50),
                speed: sp(50),
            }],
            sensor: Sensor::Cpu,
            interpolation: Interpolation::Linear,
            min_speed: sp(25),
            stop_below: None,
            ramp_rate: None,
        };
        assert!(curve.validate().is_ok());
    }

    #[test]
    fn speed_at_temp_empty_returns_min() {
        let curve = FanCurve {
            points: vec![],
            sensor: Sensor::Cpu,
            interpolation: Interpolation::Linear,
            min_speed: sp(30),
            stop_below: None,
            ramp_rate: None,
        };
        assert_eq!(curve.speed_at_temp(t(50)), 30);
    }

    #[test]
    fn speed_at_temp_stop_below() {
        let curve = FanCurve {
            points: vec![
                CurvePoint {
                    temp: t(30),
                    speed: sp(25),
                },
                CurvePoint {
                    temp: t(70),
                    speed: sp(100),
                },
            ],
            sensor: Sensor::Cpu,
            interpolation: Interpolation::Linear,
            min_speed: sp(25),
            stop_below: Some(StopBelow {
                temp: t(25),
                start_speed: sp(30),
                hysteresis: 3,
            }),
            ramp_rate: None,
        };
        // Below stop threshold: fan off
        assert_eq!(curve.speed_at_temp(t(20)), 0);
        // At threshold: normal interpolation applies
        assert_eq!(curve.speed_at_temp(t(25)), 25);
        // Above threshold: normal curve
        assert_eq!(curve.speed_at_temp(t(50)), 63);
    }

    #[test]
    fn speed_at_temp_single_point() {
        let curve = FanCurve {
            points: vec![CurvePoint {
                temp: t(50),
                speed: sp(60),
            }],
            sensor: Sensor::Cpu,
            interpolation: Interpolation::Linear,
            min_speed: sp(25),
            stop_below: None,
            ramp_rate: None,
        };
        // Below: clamp to point speed
        assert_eq!(curve.speed_at_temp(t(30)), 60);
        // At: point speed
        assert_eq!(curve.speed_at_temp(t(50)), 60);
        // Above: clamp to point speed
        assert_eq!(curve.speed_at_temp(t(70)), 60);
    }

    #[test]
    fn speed_at_temp_min_speed_clamp() {
        let curve = FanCurve {
            points: vec![
                CurvePoint {
                    temp: t(30),
                    speed: sp(10),
                },
                CurvePoint {
                    temp: t(70),
                    speed: sp(20),
                },
            ],
            sensor: Sensor::Cpu,
            interpolation: Interpolation::Linear,
            min_speed: sp(25),
            stop_below: None,
            ramp_rate: None,
        };
        // Interpolated speed (10-20) is below min_speed (25), should clamp
        assert_eq!(curve.speed_at_temp(t(30)), 25);
        assert_eq!(curve.speed_at_temp(t(50)), 25);
        assert_eq!(curve.speed_at_temp(t(70)), 25);
    }
}
