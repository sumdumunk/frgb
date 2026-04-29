use serde::{Deserialize, Serialize};

/// Type-safe brightness level (0-255).
///
/// Wraps a bare u8 to prevent confusion with speed percent, group ID, temperature,
/// and other u8 domains. Full u8 range is valid — no clamping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Brightness(u8);

impl Brightness {
    pub fn new(value: u8) -> Self {
        Self(value)
    }

    pub fn value(self) -> u8 {
        self.0
    }
}

impl std::fmt::Display for Brightness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_value() {
        assert_eq!(Brightness::new(0).value(), 0);
        assert_eq!(Brightness::new(128).value(), 128);
        assert_eq!(Brightness::new(255).value(), 255);
    }

    #[test]
    fn serde_transparent_roundtrip() {
        let b = Brightness::new(200);
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(json, "200");
        let back: Brightness = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
    }
}
