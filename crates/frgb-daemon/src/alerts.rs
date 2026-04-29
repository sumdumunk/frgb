//! Alert runner — evaluates temperature thresholds and emits alert events.
//!
//! Monitors sensor readings (shared from the curve runner's ingest path)
//! and triggers AlertEvent when a configured threshold is crossed.
//! Uses 2°C hysteresis to prevent flapping at boundary temperatures.

use std::collections::HashMap;

use frgb_model::config::{AlertConfig, AlertEvent, TempAlert};
use frgb_model::ipc::Event;
use frgb_model::sensor::Sensor;

/// Hysteresis: sensor must drop this many degrees below threshold to clear.
const HYSTERESIS_C: f32 = 2.0;

/// Tracks per-alert state for edge detection.
#[derive(Debug, Clone)]
struct AlertState {
    /// Whether this alert is currently active (threshold exceeded).
    active: bool,
}

/// Evaluates temperature alerts against cached sensor readings.
pub struct AlertRunner {
    config: Option<AlertConfig>,
    /// Per-alert firing state (indexed same as config.temp_alerts).
    states: Vec<AlertState>,
    /// Cached sensor readings (populated by `ingest`).
    sensors: HashMap<Sensor, f32>,
}

impl AlertRunner {
    pub fn new() -> Self {
        Self {
            config: None,
            states: Vec::new(),
            sensors: HashMap::new(),
        }
    }

    /// Load or replace alert configuration.
    pub fn set_config(&mut self, config: AlertConfig) {
        self.states = vec![AlertState { active: false }; config.temp_alerts.len()];
        self.config = Some(config);
    }

    /// Clear alert configuration (disables all monitoring).
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.config = None;
        self.states.clear();
    }

    /// Update cached sensor readings. Call on each poll interval.
    pub fn ingest(&mut self, sensor: &Sensor, value: f32) {
        self.sensors.insert(sensor.clone(), value);
    }

    /// Evaluate all configured alerts against current sensor readings.
    /// Returns events for newly triggered alerts.
    pub fn evaluate(&mut self) -> Vec<Event> {
        let config = match &self.config {
            Some(c) => c,
            None => return Vec::new(),
        };

        let mut events = Vec::new();

        for (i, alert) in config.temp_alerts.iter().enumerate() {
            let state = &mut self.states[i];
            let value = match self.sensors.get(&alert.sensor) {
                Some(&v) => v,
                None => continue,
            };

            let threshold = alert.threshold as f32;

            if !state.active && value >= threshold {
                // Threshold crossed — fire alert
                state.active = true;
                events.push(Event::Alert(AlertEvent {
                    sensor: alert.sensor.clone(),
                    value,
                    threshold: alert.threshold,
                }));
            } else if state.active && value < threshold - HYSTERESIS_C {
                // Dropped below hysteresis band — clear alert
                state.active = false;
            }
        }

        events
    }

    /// Collect actions from recently fired alerts.
    /// Call after `evaluate()` to get actions that need executing.
    pub fn pending_actions(&self) -> Vec<&TempAlert> {
        let config = match &self.config {
            Some(c) => c,
            None => return Vec::new(),
        };

        config
            .temp_alerts
            .iter()
            .zip(&self.states)
            .filter(|(_, state)| state.active)
            .map(|(alert, _)| alert)
            .collect()
    }

    /// Whether alert monitoring is configured.
    pub fn is_active(&self) -> bool {
        self.config.as_ref().is_some_and(|c| !c.temp_alerts.is_empty())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::config::AlertAction;

    fn test_config() -> AlertConfig {
        AlertConfig {
            temp_alerts: vec![
                TempAlert {
                    sensor: Sensor::Cpu,
                    threshold: 80,
                    action: AlertAction::Notify,
                },
                TempAlert {
                    sensor: Sensor::Gpu,
                    threshold: 90,
                    action: AlertAction::SetSpeed(frgb_model::SpeedPercent::new(100)),
                },
            ],
            fan_stall_detect: true,
            device_disconnect: true,
        }
    }

    #[test]
    fn no_config_no_events() {
        let mut runner = AlertRunner::new();
        runner.ingest(&Sensor::Cpu, 95.0);
        let events = runner.evaluate();
        assert!(events.is_empty());
        assert!(!runner.is_active());
    }

    #[test]
    fn below_threshold_no_alert() {
        let mut runner = AlertRunner::new();
        runner.set_config(test_config());
        runner.ingest(&Sensor::Cpu, 75.0);

        let events = runner.evaluate();
        assert!(events.is_empty());
    }

    #[test]
    fn crosses_threshold_fires_alert() {
        let mut runner = AlertRunner::new();
        runner.set_config(test_config());
        runner.ingest(&Sensor::Cpu, 82.0);

        let events = runner.evaluate();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Alert(e) => {
                assert_eq!(e.sensor, Sensor::Cpu);
                assert_eq!(e.threshold, 80);
                assert!((e.value - 82.0).abs() < 0.01);
            }
            other => panic!("expected Alert event, got {:?}", other),
        }
    }

    #[test]
    fn no_repeat_while_active() {
        let mut runner = AlertRunner::new();
        runner.set_config(test_config());
        runner.ingest(&Sensor::Cpu, 85.0);

        let events1 = runner.evaluate();
        assert_eq!(events1.len(), 1);

        // Still above threshold — no repeat
        runner.ingest(&Sensor::Cpu, 86.0);
        let events2 = runner.evaluate();
        assert!(events2.is_empty());
    }

    #[test]
    fn hysteresis_prevents_flapping() {
        let mut runner = AlertRunner::new();
        runner.set_config(test_config());

        // Fire alert
        runner.ingest(&Sensor::Cpu, 82.0);
        runner.evaluate();

        // Drop below threshold but within hysteresis band (80 - 2 = 78)
        runner.ingest(&Sensor::Cpu, 79.0);
        runner.evaluate();

        // Still "active" — re-crossing threshold shouldn't fire again
        runner.ingest(&Sensor::Cpu, 81.0);
        let events = runner.evaluate();
        assert!(events.is_empty(), "should not re-fire within hysteresis band");
    }

    #[test]
    fn clears_after_hysteresis() {
        let mut runner = AlertRunner::new();
        runner.set_config(test_config());

        // Fire alert
        runner.ingest(&Sensor::Cpu, 82.0);
        runner.evaluate();

        // Drop below hysteresis band (80 - 2 = 78)
        runner.ingest(&Sensor::Cpu, 77.0);
        runner.evaluate();

        // Re-cross threshold — should fire again
        runner.ingest(&Sensor::Cpu, 81.0);
        let events = runner.evaluate();
        assert_eq!(events.len(), 1, "should re-fire after clearing hysteresis");
    }

    #[test]
    fn multiple_alerts_independent() {
        let mut runner = AlertRunner::new();
        runner.set_config(test_config());

        // CPU above, GPU below
        runner.ingest(&Sensor::Cpu, 85.0);
        runner.ingest(&Sensor::Gpu, 70.0);

        let events = runner.evaluate();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Alert(e) => assert_eq!(e.sensor, Sensor::Cpu),
            other => panic!("expected CPU alert, got {:?}", other),
        }

        // Now GPU crosses too
        runner.ingest(&Sensor::Gpu, 92.0);
        let events = runner.evaluate();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Alert(e) => assert_eq!(e.sensor, Sensor::Gpu),
            other => panic!("expected GPU alert, got {:?}", other),
        }
    }

    #[test]
    fn missing_sensor_no_panic() {
        let mut runner = AlertRunner::new();
        runner.set_config(test_config());
        // No sensor readings ingested — should just skip
        let events = runner.evaluate();
        assert!(events.is_empty());
    }
}
