use serde::{Deserialize, Serialize};

/// Type-safe temperature in degrees Celsius for curve/gradient control points.
///
/// Wraps i32 for semantic clarity — distinguishes temperature setpoints from
/// other integer domains. Negative values are valid (sub-zero environments).
/// Use `.celsius()` to extract the raw value. Use `.as_f32()` for sensor comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Temperature(i32);

impl Temperature {
    pub fn new(celsius: i32) -> Self {
        Self(celsius)
    }

    pub fn celsius(self) -> i32 {
        self.0
    }

    pub fn as_f32(self) -> f32 {
        self.0 as f32
    }
}

impl std::fmt::Display for Temperature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}°C", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_celsius() {
        assert_eq!(Temperature::new(65).celsius(), 65);
        assert_eq!(Temperature::new(-10).celsius(), -10);
    }

    #[test]
    fn as_f32() {
        assert!((Temperature::new(42).as_f32() - 42.0).abs() < f32::EPSILON);
    }

    #[test]
    fn serde_transparent_roundtrip() {
        let t = Temperature::new(70);
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, "70");
        let back: Temperature = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn ordering() {
        assert!(Temperature::new(30) < Temperature::new(70));
    }
}
