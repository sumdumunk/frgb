use serde::{Deserialize, Serialize};

/// Type-safe fan speed percentage (0-100).
///
/// Wraps a bare u8, clamping to 100 on construction. Use `.value()` to extract
/// the raw byte for wire protocols and protocol encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpeedPercent(u8);

impl SpeedPercent {
    pub fn new(value: u8) -> Self {
        Self(value.min(100))
    }

    pub fn value(self) -> u8 {
        self.0
    }
}

impl std::fmt::Display for SpeedPercent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}%", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_clamps_to_100() {
        assert_eq!(SpeedPercent::new(150).value(), 100);
        assert_eq!(SpeedPercent::new(100).value(), 100);
        assert_eq!(SpeedPercent::new(50).value(), 50);
        assert_eq!(SpeedPercent::new(0).value(), 0);
    }

    #[test]
    fn serde_transparent_roundtrip() {
        let s = SpeedPercent::new(75);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "75");
        let back: SpeedPercent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn display() {
        assert_eq!(format!("{}", SpeedPercent::new(42)), "42%");
    }
}
