//! Curve runner — evaluates fan curves against sensor readings.
//!
//! Ingests SensorReadings from hwmon (or any source), caches values,
//! and applies curve-driven speed commands to groups on each poll interval.

use std::collections::HashMap;

use frgb_model::ipc::Event;
use frgb_model::sensor::Sensor;
use frgb_model::speed::{FanCurve, SpeedMode};
use frgb_model::GroupId;
use frgb_model::SpeedPercent;
use frgb_model::Temperature;

use frgb_core::System;

/// Manages curve evaluation and ramp-rate limiting per group.
pub struct CurveRunner {
    /// Per-group: active curve + last applied speed (for ramp limiting).
    curves: HashMap<GroupId, CurveState>,
    /// Cached sensor readings from last poll.
    sensors: HashMap<Sensor, f32>,
    /// Set by poll_sensors() when any reading changes; cleared by evaluate().
    sensors_dirty: bool,
}

pub struct CurveState {
    pub curve: FanCurve,
    last_speed: Option<u8>,
}

impl CurveRunner {
    pub fn new() -> Self {
        Self {
            curves: HashMap::new(),
            sensors: HashMap::new(),
            sensors_dirty: false,
        }
    }

    /// Set a curve for a group. Replaces any existing curve.
    pub fn set_curve(&mut self, group: GroupId, curve: FanCurve) {
        self.curves.insert(
            group,
            CurveState {
                curve,
                last_speed: None,
            },
        );
    }

    /// Remove a curve from a group (e.g., when switching to manual speed).
    pub fn remove_curve(&mut self, group: GroupId) {
        self.curves.remove(&group);
    }

    /// Scan device config for groups with curve-based speed modes.
    pub fn sync_from_config(&mut self, system: &System, config: &frgb_model::config::Config) {
        self.curves.clear();
        for device in system.devices() {
            if let Some(gc) = config.groups.iter().find(|g| g.id == device.group) {
                match &gc.speed {
                    SpeedMode::Curve(curve) => {
                        self.set_curve(device.group, curve.clone());
                    }
                    SpeedMode::NamedCurve(name) => {
                        if let Some(named) = config.saved_curves.iter().find(|c| c.name == *name) {
                            self.set_curve(device.group, named.curve.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Ingest sensor readings (from hwmon or any source).
    /// Returns events for sensors whose values changed (>0.5 deadband).
    pub fn ingest_readings(&mut self, readings: &[frgb_core::backend::SensorReading]) -> Vec<Event> {
        let mut events = Vec::new();

        for reading in readings {
            if let Some(sensor) = frgb_core::hwmon::classify_sensor(&reading.label) {
                let value = reading.value as f32;
                let changed = self.sensors.get(&sensor).is_none_or(|&prev| (prev - value).abs() > 0.5);
                if changed {
                    self.sensors.insert(sensor.clone(), value);
                    self.sensors_dirty = true;
                    events.push(Event::SensorUpdate { sensor, value });
                }
            }
        }

        events
    }

    /// Evaluate all active curves and return speed commands to apply.
    /// `elapsed_secs` is time since last evaluate — used to normalize ramp_rate (% per second).
    pub fn evaluate(&mut self, elapsed_secs: f32) -> (Vec<(GroupId, SpeedPercent)>, Vec<Event>) {
        if !self.sensors_dirty {
            return (Vec::new(), Vec::new());
        }
        self.sensors_dirty = false;

        let mut commands = Vec::new();
        let mut events = Vec::new();

        for (&group, state) in self.curves.iter_mut() {
            let temp = match self.sensors.get(&state.curve.sensor) {
                Some(&t) => Temperature::new(t as i32),
                None => continue,
            };

            let target = state.curve.speed_at_temp(temp);

            // Ramp rate limiting: ramp_rate is % per second, scaled by elapsed time
            let actual = if let (Some(ramp_per_sec), Some(last)) = (state.curve.ramp_rate, state.last_speed) {
                let max_change = (ramp_per_sec as f32 * elapsed_secs).round().max(1.0) as u8;
                let diff = target as i16 - last as i16;
                if diff.unsigned_abs() as u8 > max_change {
                    if diff > 0 {
                        last.saturating_add(max_change)
                    } else {
                        last.saturating_sub(max_change)
                    }
                } else {
                    target
                }
            } else {
                target
            };

            if state.last_speed != Some(actual) {
                state.last_speed = Some(actual);
                commands.push((group, SpeedPercent::new(actual)));
                events.push(Event::CurveApplied {
                    group,
                    speed: SpeedPercent::new(actual),
                    temp,
                });
            }
        }

        (commands, events)
    }

    /// Whether any curves are active.
    pub fn is_active(&self) -> bool {
        !self.curves.is_empty()
    }

    /// Expose the active curves map for read-only analysis (e.g. suggestion generation).
    pub fn active_curves(&self) -> &HashMap<GroupId, CurveState> {
        &self.curves
    }
}

#[cfg(test)]
impl CurveRunner {
    pub fn inject_sensor(&mut self, sensor: Sensor, value: f32) {
        self.sensors.insert(sensor, value);
        self.sensors_dirty = true;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::speed::{CurvePoint, Interpolation};
    use frgb_model::Temperature;

    fn sp(v: u8) -> SpeedPercent {
        SpeedPercent::new(v)
    }

    fn t(c: i32) -> Temperature {
        Temperature::new(c)
    }

    fn test_curve(sensor: Sensor) -> FanCurve {
        FanCurve {
            points: vec![
                CurvePoint {
                    temp: t(30),
                    speed: sp(30),
                },
                CurvePoint {
                    temp: t(50),
                    speed: sp(50),
                },
                CurvePoint {
                    temp: t(70),
                    speed: sp(80),
                },
                CurvePoint {
                    temp: t(90),
                    speed: sp(100),
                },
            ],
            sensor,
            interpolation: Interpolation::Linear,
            min_speed: sp(25),
            stop_below: None,
            ramp_rate: None,
        }
    }

    #[test]
    fn evaluate_no_sensors_no_commands() {
        let mut runner = CurveRunner::new();
        runner.set_curve(GroupId::new(1), test_curve(Sensor::Cpu));
        let (commands, _) = runner.evaluate(2.0);
        assert!(commands.is_empty(), "no sensor readings = no commands");
    }

    #[test]
    fn evaluate_with_sensor() {
        let mut runner = CurveRunner::new();
        runner.set_curve(GroupId::new(1), test_curve(Sensor::Cpu));
        runner.inject_sensor(Sensor::Cpu, 50.0);

        let (commands, events) = runner.evaluate(2.0);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].0, GroupId::new(1)); // group
        assert_eq!(commands[0].1, sp(50)); // speed at 50°C
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn evaluate_no_change_no_event() {
        let mut runner = CurveRunner::new();
        runner.set_curve(GroupId::new(1), test_curve(Sensor::Cpu));
        runner.inject_sensor(Sensor::Cpu, 50.0);

        runner.evaluate(2.0); // first call applies speed
        let (commands, events) = runner.evaluate(2.0); // second call: same temp, no change
        assert!(commands.is_empty());
        assert!(events.is_empty());
    }

    #[test]
    fn ramp_rate_limiting() {
        let mut curve = test_curve(Sensor::Cpu);
        curve.ramp_rate = Some(5); // 5% per second

        let mut runner = CurveRunner::new();
        runner.set_curve(GroupId::new(1), curve);
        runner.inject_sensor(Sensor::Cpu, 30.0); // speed = 30
        runner.evaluate(2.0); // sets last_speed = 30

        runner.inject_sensor(Sensor::Cpu, 90.0); // target = 100, but ramp limited
        let (commands, _) = runner.evaluate(2.0); // 5%/s × 2s = 10% max change
        assert_eq!(commands[0].1, sp(40)); // 30 + 10
    }

    #[test]
    fn remove_curve_stops_evaluation() {
        let mut runner = CurveRunner::new();
        runner.set_curve(GroupId::new(1), test_curve(Sensor::Cpu));
        assert!(runner.is_active());

        runner.remove_curve(GroupId::new(1));
        assert!(!runner.is_active());
    }
}
