use serde::{Deserialize, Serialize};

/// Type-safe group identifier for device groups (1-8 typical, protocol allows 1-254).
///
/// Wraps a bare u8 to prevent confusion with brightness, speed, temperature,
/// and other u8 domains. Use `.value()` to extract the raw byte for wire protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(u8);

impl GroupId {
    pub fn new(value: u8) -> Self {
        Self(value)
    }

    pub fn value(self) -> u8 {
        self.0
    }
}

impl std::fmt::Display for GroupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u8> for GroupId {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_value_roundtrip() {
        let g = GroupId::new(3);
        assert_eq!(g.value(), 3);
    }

    #[test]
    fn from_u8() {
        let g: GroupId = 5u8.into();
        assert_eq!(g.value(), 5);
    }

    #[test]
    fn display() {
        let g = GroupId::new(7);
        assert_eq!(format!("{g}"), "7");
    }

    #[test]
    fn serde_transparent_roundtrip() {
        let g = GroupId::new(4);
        let json = serde_json::to_string(&g).unwrap();
        assert_eq!(json, "4"); // transparent — not {"0":4}
        let back: GroupId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, g);
    }

    #[test]
    fn hash_and_eq() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(GroupId::new(1));
        set.insert(GroupId::new(1));
        set.insert(GroupId::new(2));
        assert_eq!(set.len(), 2);
    }
}
