//! Builds sensor graph data for the sensors page.
//!
//! This module is read-only — no user callbacks to wire.
//! A 1-second timer in main.rs calls [`build_sensor_graph_data`] and pushes
//! the result into the UI.

use slint::{ModelRc, SharedString, VecModel};

use crate::sensor_history::SensorHistory;
use crate::{SensorGraphData, SensorGraphPoint};

/// Returns an (r, g, b) color tuple for a given sensor label.
fn sensor_color(label: &str) -> (i32, i32, i32) {
    match label {
        "CPU" => (255, 99, 71),    // tomato red
        "GPU" => (50, 205, 50),    // lime green
        "Water" => (74, 158, 255), // blue
        _ => (200, 200, 200),      // gray
    }
}

/// Alert thresholds keyed by sensor label. Shared between fetch and graph builder.
pub type AlertThresholds = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, f32>>>;

/// Convert the current [`SensorHistory`] snapshot into a `Vec<SensorGraphData>`
/// suitable for pushing into the Slint UI.
pub fn build_sensor_graph_data(history: &SensorHistory, thresholds: &AlertThresholds) -> Vec<SensorGraphData> {
    let labels = history.labels();
    let thresh = thresholds.lock().unwrap();
    let mut out = Vec::with_capacity(labels.len());

    for label in &labels {
        let Some(series) = history.get(label) else {
            continue;
        };
        if series.is_empty() {
            continue;
        }

        let readings = series.readings();
        let len = readings.len() as f32;
        let (color_r, color_g, color_b) = sensor_color(label);

        let points: Vec<SensorGraphPoint> = readings
            .iter()
            .enumerate()
            .map(|(i, &v)| SensorGraphPoint {
                x: i as f32 / len,
                y: v,
            })
            .collect();

        let unit = sensor_unit(label);
        out.push(SensorGraphData {
            label: SharedString::from(label.as_str()),
            unit: SharedString::from(unit),
            current: series.current().unwrap_or(-999.0),
            min_val: series.min(),
            max_val: series.max(),
            points: ModelRc::new(VecModel::from(points)),
            color_r,
            color_g,
            color_b,
            alert_threshold: *thresh.get(label.as_str()).unwrap_or(&0.0),
        });
    }

    out
}

fn sensor_unit(label: &str) -> &'static str {
    let l = label.to_lowercase();
    if l.contains("power") || l.contains("watt") {
        "W"
    } else if l.contains("usage") || l.contains("util") || l.contains("load") {
        "%"
    } else {
        "°C"
    }
}
