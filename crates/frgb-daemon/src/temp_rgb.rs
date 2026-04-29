//! Temperature-reactive RGB runner — maps sensor temperature to gradient colors.
//!
//! Evaluates TempRgbConfig against sensor readings and applies interpolated
//! static colors to device groups. Runs on the same poll interval as fan curves.

use std::collections::HashMap;

use frgb_model::ipc::Event;
use frgb_model::rgb::{Rgb, RgbMode, TempRgbConfig};
use frgb_model::sensor::Sensor;
use frgb_model::GroupId;

/// Manages per-group temperature-reactive RGB state.
pub struct TempRgbRunner {
    /// Per-group: config + last applied color (for change detection).
    configs: HashMap<GroupId, TempRgbState>,
    /// Cached sensor readings.
    sensors: HashMap<Sensor, f32>,
    sensors_dirty: bool,
}

struct TempRgbState {
    config: TempRgbConfig,
    last_color: Option<Rgb>,
}

impl TempRgbRunner {
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
            sensors: HashMap::new(),
            sensors_dirty: false,
        }
    }

    /// Activate temperature-reactive RGB for a group.
    pub fn set(&mut self, group: GroupId, config: TempRgbConfig) {
        self.configs.insert(
            group,
            TempRgbState {
                config,
                last_color: None,
            },
        );
    }

    /// Deactivate temperature-reactive RGB for a group.
    #[allow(dead_code)]
    pub fn remove(&mut self, group: GroupId) {
        self.configs.remove(&group);
    }

    /// Update cached sensor reading.
    pub fn ingest(&mut self, sensor: &Sensor, value: f32) {
        self.sensors.insert(sensor.clone(), value);
        self.sensors_dirty = true;
    }

    /// Evaluate all active configs and return RGB commands to apply.
    /// Returns (group, RgbMode) pairs for groups whose color changed.
    pub fn evaluate(&mut self) -> (Vec<(GroupId, RgbMode)>, Vec<Event>) {
        if !self.sensors_dirty {
            return (Vec::new(), Vec::new());
        }
        self.sensors_dirty = false;

        let mut commands = Vec::new();
        let mut events = Vec::new();

        for (&group, state) in self.configs.iter_mut() {
            let temp = match self.sensors.get(&state.config.sensor) {
                Some(&t) => t,
                None => continue,
            };

            let color = interpolate_gradient(&state.config.gradient, temp);

            if state.last_color != Some(color) {
                state.last_color = Some(color);
                let mode = RgbMode::Static {
                    ring: state.config.ring,
                    color,
                    brightness: frgb_model::Brightness::new(255),
                };
                events.push(Event::RgbChanged {
                    group,
                    mode: mode.clone(),
                });
                commands.push((group, mode));
            }
        }

        (commands, events)
    }

    /// Sync from config: scan for groups with TempRgb RGB mode.
    pub fn sync_from_config(&mut self, config: &frgb_model::config::Config) {
        self.configs.clear();
        // Scan group configs for TempRgb RGB modes
        for gc in &config.groups {
            if let RgbMode::TempRgb(ref tr) = gc.rgb {
                self.set(gc.id, tr.clone());
            }
        }
    }

    /// Whether any temperature-reactive RGB configs are active.
    pub fn is_active(&self) -> bool {
        !self.configs.is_empty()
    }
}

/// Interpolate a color from a temperature gradient.
///
/// The gradient is a sorted list of (temp, color) points. Values below
/// the first point clamp to its color; values above the last clamp similarly.
/// Between points, colors are linearly interpolated per channel.
fn interpolate_gradient(gradient: &[frgb_model::rgb::TempColorPoint], temp: f32) -> Rgb {
    if gradient.is_empty() {
        return Rgb { r: 0, g: 0, b: 0 };
    }
    if gradient.len() == 1 || temp <= gradient[0].temp.as_f32() {
        return gradient[0].color;
    }
    if temp >= gradient.last().unwrap().temp.as_f32() {
        return gradient.last().unwrap().color;
    }

    // Find the two surrounding points
    for i in 0..gradient.len() - 1 {
        let lo = &gradient[i];
        let hi = &gradient[i + 1];
        let lo_t = lo.temp.as_f32();
        let hi_t = hi.temp.as_f32();

        if temp >= lo_t && temp <= hi_t {
            let t = (temp - lo_t) / (hi_t - lo_t);
            return lerp_color(&lo.color, &hi.color, t);
        }
    }

    gradient.last().unwrap().color
}

fn lerp_color(a: &Rgb, b: &Rgb, t: f32) -> Rgb {
    Rgb {
        r: (a.r as f32 + (b.r as f32 - a.r as f32) * t).round() as u8,
        g: (a.g as f32 + (b.g as f32 - a.g as f32) * t).round() as u8,
        b: (a.b as f32 + (b.b as f32 - a.b as f32) * t).round() as u8,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::rgb::{Ring, TempColorPoint};
    use frgb_model::Temperature;

    fn t(c: i32) -> Temperature {
        Temperature::new(c)
    }

    fn test_gradient() -> Vec<TempColorPoint> {
        vec![
            TempColorPoint {
                temp: t(30),
                color: Rgb { r: 0, g: 0, b: 255 },
            }, // cold: blue
            TempColorPoint {
                temp: t(50),
                color: Rgb { r: 0, g: 255, b: 0 },
            }, // warm: green
            TempColorPoint {
                temp: t(80),
                color: Rgb { r: 255, g: 0, b: 0 },
            }, // hot: red
        ]
    }

    fn test_config() -> TempRgbConfig {
        TempRgbConfig {
            sensor: Sensor::Cpu,
            gradient: test_gradient(),
            ring: Ring::Both,
        }
    }

    #[test]
    fn interpolate_below_range_clamps() {
        let g = test_gradient();
        let c = interpolate_gradient(&g, 10.0);
        assert_eq!(c, Rgb { r: 0, g: 0, b: 255 });
    }

    #[test]
    fn interpolate_above_range_clamps() {
        let g = test_gradient();
        let c = interpolate_gradient(&g, 100.0);
        assert_eq!(c, Rgb { r: 255, g: 0, b: 0 });
    }

    #[test]
    fn interpolate_exact_point() {
        let g = test_gradient();
        let c = interpolate_gradient(&g, 50.0);
        assert_eq!(c, Rgb { r: 0, g: 255, b: 0 });
    }

    #[test]
    fn interpolate_midpoint() {
        let g = test_gradient();
        // Midpoint between blue (30°C) and green (50°C) at 40°C
        let c = interpolate_gradient(&g, 40.0);
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 128); // 0 + (255-0)*0.5 = 127.5 → 128
        assert_eq!(c.b, 128); // 255 + (0-255)*0.5 = 127.5 → 128
    }

    #[test]
    fn empty_gradient_returns_black() {
        let c = interpolate_gradient(&[], 50.0);
        assert_eq!(c, Rgb { r: 0, g: 0, b: 0 });
    }

    #[test]
    fn evaluate_no_sensors_no_commands() {
        let mut runner = TempRgbRunner::new();
        runner.set(GroupId::new(1), test_config());
        let (commands, _) = runner.evaluate();
        assert!(commands.is_empty());
    }

    #[test]
    fn evaluate_produces_rgb_command() {
        let mut runner = TempRgbRunner::new();
        runner.set(GroupId::new(1), test_config());
        runner.ingest(&Sensor::Cpu, 50.0);

        let (commands, events) = runner.evaluate();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].0, GroupId::new(1)); // group
        match &commands[0].1 {
            RgbMode::Static {
                color,
                ring,
                brightness,
            } => {
                assert_eq!(*color, Rgb { r: 0, g: 255, b: 0 });
                assert_eq!(*ring, Ring::Both);
                assert_eq!(*brightness, frgb_model::Brightness::new(255));
            }
            other => panic!("expected Static, got {:?}", other),
        }
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn no_change_no_event() {
        let mut runner = TempRgbRunner::new();
        runner.set(GroupId::new(1), test_config());
        runner.ingest(&Sensor::Cpu, 50.0);
        runner.evaluate(); // first apply

        runner.ingest(&Sensor::Cpu, 50.0);
        let (commands, events) = runner.evaluate();
        assert!(commands.is_empty());
        assert!(events.is_empty());
    }

    #[test]
    fn remove_stops_evaluation() {
        let mut runner = TempRgbRunner::new();
        runner.set(GroupId::new(1), test_config());
        assert!(runner.is_active());

        runner.remove(GroupId::new(1));
        assert!(!runner.is_active());
    }
}
