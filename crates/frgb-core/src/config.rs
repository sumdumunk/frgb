use crate::error::{CoreError, Result};
use frgb_model::config::Config;
use std::path::PathBuf;

/// Default config directory: `$XDG_CONFIG_HOME/frgb/` or `~/.config/frgb/`.
pub fn config_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| CoreError::Config("could not determine config directory".into()))?
        .join("frgb");
    Ok(dir)
}

/// Default config file path.
pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

/// Current config schema version. Increment when making breaking changes.
const CURRENT_CONFIG_VERSION: u32 = 1;

/// Load config from the default path. Returns `Config::default()` if file doesn't exist.
/// Checks version and runs migrations if needed.
pub fn load_config() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config::default());
    }
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| CoreError::Config(format!("failed to read {}: {e}", path.display())))?;
    let mut config: Config = serde_json::from_str(&contents)
        .map_err(|e| CoreError::Config(format!("failed to parse {}: {e}", path.display())))?;
    migrate(&mut config);
    Ok(config)
}

/// Run config migrations from the file's version to current.
/// Each migration step handles one version bump. If the version is already
/// current, this is a no-op. Unknown future versions log a warning.
fn migrate(config: &mut Config) {
    if config.version == CURRENT_CONFIG_VERSION {
        return;
    }
    if config.version > CURRENT_CONFIG_VERSION {
        tracing::warn!(
            "Config version {} is newer than supported ({}). Some fields may be ignored.",
            config.version,
            CURRENT_CONFIG_VERSION
        );
        return;
    }
    // Future migrations go here:
    // if config.version < 2 { migrate_v1_to_v2(config); }
    // if config.version < 3 { migrate_v2_to_v3(config); }
    config.version = CURRENT_CONFIG_VERSION;
}

/// Save config to the default path, creating directories as needed.
pub fn save_config(config: &Config) -> Result<()> {
    let path = config_path()?;
    save_config_to(config, &path)
}

/// Load config from a specific path.
pub fn load_config_from(path: &std::path::Path) -> Result<Config> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| CoreError::Config(format!("failed to read {}: {e}", path.display())))?;
    let config: Config = serde_json::from_str(&contents)
        .map_err(|e| CoreError::Config(format!("failed to parse {}: {e}", path.display())))?;
    Ok(config)
}

/// Save config to a specific path (atomic: write temp file, then rename).
pub fn save_config_to(config: &Config, path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CoreError::Config(format!("failed to create {}: {e}", parent.display())))?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| CoreError::Config(format!("failed to serialize config: {e}")))?;

    // Write to temp file in same directory, then atomic rename
    let tmp_path = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp_path, &json)
        .map_err(|e| CoreError::Config(format!("failed to write {}: {e}", tmp_path.display())))?;
    std::fs::rename(&tmp_path, path)
        .map_err(|e| CoreError::Config(format!("failed to rename {}: {e}", tmp_path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn config_roundtrip_to_file() {
        let dir = std::env::temp_dir().join(format!("frgb_test_config_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("test_config.json");

        let config = Config::default();
        save_config_to(&config, &path).unwrap();

        let loaded = load_config_from(&path).unwrap();
        assert_eq!(loaded.daemon.poll_interval_ms, config.daemon.poll_interval_ms);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_config_from_missing_file_fails() {
        let path = std::path::Path::new("/tmp/frgb_nonexistent_config.json");
        assert!(load_config_from(path).is_err());
    }

    #[test]
    fn migrate_current_version_is_noop() {
        let mut config = Config { version: CURRENT_CONFIG_VERSION, ..Default::default() };
        migrate(&mut config);
        assert_eq!(config.version, CURRENT_CONFIG_VERSION);
    }

    #[test]
    fn migrate_future_version_logs_warning() {
        let mut config = Config { version: 99, ..Default::default() };
        migrate(&mut config);
        // Version is not downgraded — just warned
        assert_eq!(config.version, 99);
    }

    #[test]
    fn load_config_from_invalid_json_fails() {
        let dir = std::env::temp_dir().join(format!("frgb_test_config_bad_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.json");
        fs::write(&path, "not json").unwrap();

        assert!(load_config_from(&path).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
