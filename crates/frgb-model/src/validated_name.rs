use serde::{Deserialize, Serialize};

/// A validated string name (1-64 characters, non-empty).
///
/// Used for profile names, curve names, effect names, preset names, and other
/// user-facing identifiers. Validation happens on construction — invalid names
/// produce errors, not silent truncation.
///
/// Serde `try_from` ensures validation on deserialization: config files with
/// invalid names produce parse errors instead of silently loading.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ValidatedName(String);

impl ValidatedName {
    const MAX_LEN: usize = 64;

    pub fn new(s: impl Into<String>) -> Result<Self, String> {
        let s = s.into();
        if s.is_empty() {
            return Err("name cannot be empty".into());
        }
        if s.len() > Self::MAX_LEN {
            return Err(format!("name too long ({} chars, max {})", s.len(), Self::MAX_LEN));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ValidatedName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ValidatedName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<ValidatedName> for String {
    fn from(v: ValidatedName) -> String {
        v.0
    }
}

impl<'de> Deserialize<'de> for ValidatedName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        ValidatedName::new(s).map_err(serde::de::Error::custom)
    }
}

impl PartialEq<str> for ValidatedName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for ValidatedName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for ValidatedName {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}

impl PartialEq<ValidatedName> for str {
    fn eq(&self, other: &ValidatedName) -> bool {
        self == other.0
    }
}

impl PartialEq<ValidatedName> for String {
    fn eq(&self, other: &ValidatedName) -> bool {
        *self == other.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_name() {
        let n = ValidatedName::new("test").unwrap();
        assert_eq!(n.as_str(), "test");
    }

    #[test]
    fn empty_name_rejected() {
        assert!(ValidatedName::new("").is_err());
    }

    #[test]
    fn too_long_rejected() {
        let long = "a".repeat(65);
        assert!(ValidatedName::new(long).is_err());
    }

    #[test]
    fn max_length_ok() {
        let max = "a".repeat(64);
        assert!(ValidatedName::new(max).is_ok());
    }

    #[test]
    fn serde_roundtrip() {
        let n = ValidatedName::new("my-profile").unwrap();
        let json = serde_json::to_string(&n).unwrap();
        assert_eq!(json, "\"my-profile\"");
        let back: ValidatedName = serde_json::from_str(&json).unwrap();
        assert_eq!(back, n);
    }

    #[test]
    fn serde_rejects_empty() {
        let result: Result<ValidatedName, _> = serde_json::from_str("\"\"");
        assert!(result.is_err());
    }

    #[test]
    fn string_comparison() {
        let n = ValidatedName::new("test").unwrap();
        assert!(n == "test");                  // PartialEq<&str>
        assert!(n == *"test");                 // PartialEq<str>
        let s = String::from("test");
        assert!(n == s);                       // PartialEq<String>
        assert!(s == n);                       // PartialEq<ValidatedName> for String
    }
}
