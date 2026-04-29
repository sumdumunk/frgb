use crate::error::{CoreError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const STATE_FILE_VERSION: u32 = 1;

/// A single channel's pre-takeover snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateEntry {
    pub pwm: u8,
    pub saved_enable: u8,
    #[serde(default)]
    pub offloaded: bool,
}

/// On-disk state file format. See spec §7.1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateFile {
    pub version: u32,
    pub chip_name: String,
    pub chip_path_hint: String,
    pub entries: Vec<StateEntry>,
}

/// In-memory snapshot of channels we've taken over this session. Drives both
/// the state file and the clean-shutdown restore path.
#[derive(Debug, Default)]
pub struct PendingRestores {
    entries: BTreeMap<u8, StateEntry>,
}

impl PendingRestores {
    /// Record a snapshot. First call wins — subsequent calls are ignored so
    /// repeated `ensure_snapshot` invocations don't clobber the original
    /// value with whatever we just wrote.
    pub fn record(&mut self, pwm: u8, saved_enable: u8, offloaded: bool) {
        self.entries
            .entry(pwm)
            .or_insert(StateEntry { pwm, saved_enable, offloaded });
    }

    pub fn remove(&mut self, pwm: u8) {
        self.entries.remove(&pwm);
    }

    pub fn contains(&self, pwm: u8) -> bool {
        self.entries.contains_key(&pwm)
    }

    pub fn get(&self, pwm: u8) -> Option<&StateEntry> {
        self.entries.get(&pwm)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &StateEntry> {
        self.entries.values()
    }

    pub fn to_file(&self, chip_name: &str, chip_path_hint: &str) -> StateFile {
        StateFile {
            version: STATE_FILE_VERSION,
            chip_name: chip_name.to_string(),
            chip_path_hint: chip_path_hint.to_string(),
            entries: self.entries.values().cloned().collect(),
        }
    }

    /// Called from shutdown after the saved enables have been written back
    /// and the state file deleted. Clears the map so subsequent Drop calls
    /// don't re-run the work.
    pub(crate) fn clear_for_shutdown(&mut self) {
        self.entries.clear();
    }
}

/// Atomic save: write to `{path}.tmp`, fsync the data to disk, then rename
/// over the target. The fsync matters in the panic-hook / crash-recovery path
/// where we may be racing a power loss.
pub fn save_state_file(state: &StateFile, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CoreError::Config(format!("hwmon state mkdir {}: {e}", parent.display())))?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| CoreError::Config(format!("hwmon state serialize: {e}")))?;
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| CoreError::Config(format!("hwmon state create {}: {e}", tmp.display())))?;
        f.write_all(json.as_bytes())
            .map_err(|e| CoreError::Config(format!("hwmon state write {}: {e}", tmp.display())))?;
        f.sync_all()
            .map_err(|e| CoreError::Config(format!("hwmon state fsync {}: {e}", tmp.display())))?;
    }
    std::fs::rename(&tmp, path)
        .map_err(|e| CoreError::Config(format!("hwmon state rename {}: {e}", tmp.display())))?;
    Ok(())
}

/// Load state file. Returns an error if missing or unparseable — callers that
/// want "file missing = empty state" should use `load_state_file_optional`.
pub fn load_state_file(path: &Path) -> Result<StateFile> {
    let data = std::fs::read_to_string(path)
        .map_err(|e| CoreError::Config(format!("hwmon state read {}: {e}", path.display())))?;
    serde_json::from_str(&data)
        .map_err(|e| CoreError::Config(format!("hwmon state parse {}: {e}", path.display())))
}

/// `load_state_file` that returns `None` on missing / unparseable, logging
/// the error. Used at startup where "can't restore — continue clean" is
/// the desired behavior.
pub fn load_state_file_optional(path: &Path) -> Option<StateFile> {
    if !path.exists() {
        return None;
    }
    match load_state_file(path) {
        Ok(sf) => Some(sf),
        Err(e) => {
            tracing::warn!("hwmon: state file unreadable, starting clean: {e}");
            None
        }
    }
}

/// Default state-file path: `$XDG_STATE_HOME/frgb/hwmon-saved.json`, falling
/// back to `~/.local/state/frgb/hwmon-saved.json`. Tests drive the internal
/// `default_state_path_for` so we can verify both arms without env-mucking.
pub fn default_state_path() -> PathBuf {
    let xdg = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    default_state_path_for(xdg, home)
}

pub fn default_state_path_for(
    xdg_state_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PathBuf {
    if let Some(xdg) = xdg_state_home {
        return xdg.join("frgb").join("hwmon-saved.json");
    }
    let home = home.unwrap_or_else(|| PathBuf::from("/"));
    home.join(".local/state/frgb/hwmon-saved.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("frgb_hwstate_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hwmon-saved.json");

        let original = StateFile {
            version: STATE_FILE_VERSION,
            chip_name: "nct6799".into(),
            chip_path_hint: "/sys/class/hwmon/hwmon3".into(),
            entries: vec![
                StateEntry { pwm: 2, saved_enable: 5, offloaded: false },
                StateEntry { pwm: 7, saved_enable: 5, offloaded: true },
            ],
        };
        save_state_file(&original, &path).unwrap();
        let loaded = load_state_file(&path).unwrap();
        assert_eq!(loaded, original);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_file_returns_none_cleanly() {
        let path = std::path::Path::new("/tmp/frgb_nonexistent_hwmon_state_xyz.json");
        assert!(load_state_file_optional(path).is_none());
    }

    #[test]
    fn ensure_snapshot_records_once() {
        let mut snap = PendingRestores::default();
        assert_eq!(snap.len(), 0);
        snap.record(2, 5, false);
        assert_eq!(snap.len(), 1);
        // Second call is idempotent
        snap.record(2, 3, false);
        assert_eq!(snap.len(), 1);
        // First recorded value wins
        assert_eq!(snap.get(2).unwrap().saved_enable, 5);
    }

    #[test]
    fn remove_clears_entry() {
        let mut snap = PendingRestores::default();
        snap.record(2, 5, false);
        snap.record(3, 5, false);
        snap.remove(2);
        assert_eq!(snap.len(), 1);
        assert!(snap.get(2).is_none());
    }

    #[test]
    fn to_file_preserves_entries() {
        let mut snap = PendingRestores::default();
        snap.record(2, 5, false);
        snap.record(7, 5, true);
        let file = snap.to_file("nct6799", "/sys/class/hwmon/hwmon3");
        assert_eq!(file.chip_name, "nct6799");
        assert_eq!(file.entries.len(), 2);
    }

    #[test]
    fn default_state_path_uses_xdg_state_home_when_set() {
        let tmp = std::env::temp_dir().join("xdg_state_test");
        let path = default_state_path_for(Some(tmp.clone()), None);
        assert_eq!(path, tmp.join("frgb").join("hwmon-saved.json"));
    }

    #[test]
    fn default_state_path_falls_back_to_home_local_state() {
        let home = std::env::temp_dir().join("home_test");
        let path = default_state_path_for(None, Some(home.clone()));
        assert_eq!(path, home.join(".local/state/frgb/hwmon-saved.json"));
    }
}
