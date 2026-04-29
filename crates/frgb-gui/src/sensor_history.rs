use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Maximum number of data points stored per sensor (5 minutes at 1 sample/sec).
const MAX_POINTS: usize = 300;

/// A fixed-capacity ring buffer of f32 temperature readings.
#[derive(Clone)]
pub struct SensorTimeSeries {
    buf: Vec<f32>,
    head: usize, // index where the next write goes
    len: usize,
}

impl SensorTimeSeries {
    pub fn new() -> Self {
        Self {
            buf: vec![0.0; MAX_POINTS],
            head: 0,
            len: 0,
        }
    }

    /// Append a new reading, overwriting the oldest entry once capacity is reached.
    pub fn push(&mut self, value: f32) {
        self.buf[self.head] = value;
        self.head = (self.head + 1) % MAX_POINTS;
        if self.len < MAX_POINTS {
            self.len += 1;
        }
    }

    /// Return all stored readings in chronological order (oldest first).
    pub fn readings(&self) -> Vec<f32> {
        if self.len == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(self.len);
        if self.len < MAX_POINTS {
            // Buffer not yet full; data starts at index 0.
            out.extend_from_slice(&self.buf[..self.len]);
        } else {
            // Buffer is full; oldest entry is at `head`.
            out.extend_from_slice(&self.buf[self.head..]);
            out.extend_from_slice(&self.buf[..self.head]);
        }
        out
    }

    /// Most recent reading, or `None` if no data has been recorded.
    pub fn current(&self) -> Option<f32> {
        if self.len == 0 {
            return None;
        }
        // head points one past the last write; wrap back.
        let last = (self.head + MAX_POINTS - 1) % MAX_POINTS;
        Some(self.buf[last])
    }

    /// Minimum value across all stored readings, or `f32::INFINITY` if empty.
    pub fn min(&self) -> f32 {
        self.readings().into_iter().fold(f32::INFINITY, f32::min)
    }

    /// Maximum value across all stored readings, or `f32::NEG_INFINITY` if empty.
    pub fn max(&self) -> f32 {
        self.readings().into_iter().fold(f32::NEG_INFINITY, f32::max)
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for SensorTimeSeries {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SensorHistory — shared, multi-sensor store
// ---------------------------------------------------------------------------

/// Thread-safe store of per-sensor time-series data.
#[derive(Clone)]
pub struct SensorHistory(Arc<Mutex<HashMap<String, SensorTimeSeries>>>);

impl SensorHistory {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::new())))
    }

    /// Record a reading for the named sensor.
    pub fn record(&self, label: &str, value: f32) {
        let mut map = self.0.lock().expect("SensorHistory lock poisoned");
        map.entry(label.to_owned()).or_default().push(value);
    }

    /// Return a cloned snapshot of the named sensor's series, or `None` if unknown.
    pub fn get(&self, label: &str) -> Option<SensorTimeSeries> {
        let map = self.0.lock().expect("SensorHistory lock poisoned");
        map.get(label).cloned()
    }

    /// All sensor labels that have at least one reading.
    pub fn labels(&self) -> Vec<String> {
        let map = self.0.lock().expect("SensorHistory lock poisoned");
        let mut labels: Vec<String> = map.keys().cloned().collect();
        labels.sort();
        labels
    }
}

impl Default for SensorHistory {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_read() {
        let mut s = SensorTimeSeries::new();
        s.push(10.0);
        s.push(20.0);
        s.push(30.0);

        assert_eq!(s.len(), 3);
        assert_eq!(s.readings(), vec![10.0, 20.0, 30.0]);
        assert_eq!(s.current(), Some(30.0));
        assert_eq!(s.min(), 10.0);
        assert_eq!(s.max(), 30.0);
    }

    #[test]
    fn wraps_around() {
        let mut s = SensorTimeSeries::new();
        // Fill beyond capacity; oldest entries should be discarded.
        for i in 0..(MAX_POINTS + 5) {
            s.push(i as f32);
        }

        assert_eq!(s.len(), MAX_POINTS);

        let readings = s.readings();
        assert_eq!(readings.len(), MAX_POINTS);

        // The oldest retained value should be 5 (the first five were overwritten).
        assert_eq!(readings[0], 5.0);
        // The newest value should be MAX_POINTS + 4.
        assert_eq!(*readings.last().unwrap(), (MAX_POINTS + 4) as f32);
        assert_eq!(s.current(), Some((MAX_POINTS + 4) as f32));
    }

    #[test]
    fn history_records() {
        let h = SensorHistory::new();

        h.record("cpu", 55.0);
        h.record("cpu", 60.0);
        h.record("gpu", 70.0);

        let labels = h.labels();
        assert_eq!(labels, vec!["cpu", "gpu"]);

        let cpu = h.get("cpu").unwrap();
        assert_eq!(cpu.len(), 2);
        assert_eq!(cpu.current(), Some(60.0));

        let gpu = h.get("gpu").unwrap();
        assert_eq!(gpu.len(), 1);
        assert_eq!(gpu.current(), Some(70.0));

        assert!(h.get("nonexistent").is_none());
    }
}
