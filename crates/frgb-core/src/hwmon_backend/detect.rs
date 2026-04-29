use crate::hwmon_backend::fs::HwmonFs;
use std::path::{Path, PathBuf};

/// Highest `pwmN` index to probe during channel enumeration.
///
/// Set to 8 rather than 7 (nct6799's stated maximum) to be future-safe for
/// unreleased Nuvoton / ITE revisions exposing one additional channel.
/// Probing an absent index is a no-op — `path_exists` returns false and
/// the channel is skipped — so the overhead is negligible.
const PWM_CHANNEL_PROBE_LIMIT: u8 = 8;

/// Chip name prefixes this backend will try to drive. See spec §4.1.
/// `asus` entries are listed for completeness (sensor coexistence) but the
/// channel enumerator won't find any `pwmN` nodes on them, so they collapse
/// to zero-channel backends.
const SUPPORTED_PREFIXES: &[&str] = &[
    "nct67", "it87", "it86", "w83", "asus-ec", "asus_wmi_sensors",
];

/// A channel (pwmN + fanN_input pair) on a detected chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInfo {
    pub pwm: u8,
    pub has_enable: bool,
    pub has_smart_fan_iv: bool,   // pwmN_auto_point1_temp present
    pub has_temp_sel: bool,
    pub has_floor: bool,
}

/// A chip selected by `detect_chip`. `path` is the `/sys/class/hwmon/hwmonN/`
/// directory we will read/write through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedChip {
    pub name: String,                // "nct6799" or "nct6799-isa-0290"
    pub path: PathBuf,
    pub channels: Vec<ChannelInfo>,
}

/// Return whether `name` is a supported chip family.
pub fn is_supported(name: &str) -> bool {
    SUPPORTED_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Scan a hwmon base directory and pick the best-fit chip.
/// Returns `None` when no supported chip exposes any writable pwmN node.
/// Selection rule (see spec §4.1): most pwmN nodes wins; ties broken by
/// lowest hwmonN index (lexicographic path sort).
pub fn detect_chip<F: HwmonFs + ?Sized>(fs: &F, base: &Path) -> Option<DetectedChip> {
    let mut entries = fs.read_dir(base).ok()?;
    entries.sort();

    let mut candidates: Vec<DetectedChip> = Vec::new();
    for chip_path in entries {
        let name_path = chip_path.join("name");
        let Ok(raw) = fs.read_to_string(&name_path) else { continue };
        let name = raw.trim().to_string();
        if !is_supported(&name) {
            continue;
        }
        let channels = enumerate_channels(fs, &chip_path);
        if channels.is_empty() {
            continue;
        }
        candidates.push(DetectedChip { name, path: chip_path, channels });
    }

    // Prefer the chip with the most writable pwmN nodes; ties → lowest path
    // lexicographically. Matches numeric `hwmonN` order for hwmon0..hwmon9
    // (the common case). On systems with 10+ hwmon nodes, `"hwmon10"` sorts
    // before `"hwmon2"`, which means we may pick a higher-numbered chip as the
    // tie-break winner — accepted as a rare-case edge.
    candidates.sort_by(|a, b| {
        b.channels.len().cmp(&a.channels.len())
            .then_with(|| a.path.cmp(&b.path))
    });
    let chosen = candidates.into_iter().next()?;
    Some(chosen)
}

/// Enumerate channels on a specific chip directory. A channel qualifies when
/// BOTH `pwmN` and `fanN_input` exist. Optional feature nodes are recorded
/// per-channel so Phase 2 can check them without another scan.
fn enumerate_channels<F: HwmonFs + ?Sized>(fs: &F, chip_path: &Path) -> Vec<ChannelInfo> {
    let mut out = Vec::new();
    for n in 1..=PWM_CHANNEL_PROBE_LIMIT {
        let pwm_path = chip_path.join(format!("pwm{n}"));
        let fan_path = chip_path.join(format!("fan{n}_input"));
        if !fs.path_exists(&pwm_path) || !fs.path_exists(&fan_path) {
            continue;
        }
        out.push(ChannelInfo {
            pwm: n,
            has_enable: fs.path_exists(&chip_path.join(format!("pwm{n}_enable"))),
            has_smart_fan_iv: fs.path_exists(&chip_path.join(format!("pwm{n}_auto_point1_temp"))),
            has_temp_sel: fs.path_exists(&chip_path.join(format!("pwm{n}_temp_sel"))),
            has_floor: fs.path_exists(&chip_path.join(format!("pwm{n}_floor"))),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hwmon_backend::fs::HwmonFs;
    use std::collections::HashMap;
    use std::io;
    use std::path::{Path, PathBuf};

    /// In-memory fake fs backed by a map: path -> file contents.
    /// Any path not in the map is treated as a directory (for read_dir) or
    /// returned as NotFound (for reads). `write_str` errors for unknown paths
    /// so tests fail loudly if something touches a file we didn't set up.
    #[derive(Default)]
    struct FakeFs {
        files: HashMap<PathBuf, String>,
        dirs: HashMap<PathBuf, Vec<PathBuf>>,
    }

    impl FakeFs {
        fn set_file(&mut self, path: impl Into<PathBuf>, contents: impl Into<String>) {
            let path = path.into();
            self.files.insert(path.clone(), contents.into());
            if let Some(parent) = path.parent() {
                self.dirs
                    .entry(parent.to_path_buf())
                    .or_default()
                    .push(path);
            }
        }
        fn set_dir(&mut self, path: impl Into<PathBuf>, children: Vec<PathBuf>) {
            self.dirs.insert(path.into(), children);
        }
    }

    impl HwmonFs for FakeFs {
        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{path:?}")))
        }
        fn write_str(&self, path: &Path, _contents: &str) -> io::Result<()> {
            if !self.files.contains_key(path) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "unexpected write"));
            }
            Ok(())
        }
        fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
            self.dirs
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{path:?}")))
        }
        fn path_exists(&self, path: &Path) -> bool {
            self.files.contains_key(path) || self.dirs.contains_key(path)
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    fn nct6799_with_2_channels(fs: &mut FakeFs) {
        fs.set_dir(
            PathBuf::from("/sys/class/hwmon"),
            vec![PathBuf::from("/sys/class/hwmon/hwmon3")],
        );
        fs.set_file("/sys/class/hwmon/hwmon3/name", "nct6799\n");
        for n in [2u8, 7u8] {
            fs.set_file(format!("/sys/class/hwmon/hwmon3/pwm{n}"), "0\n");
            fs.set_file(format!("/sys/class/hwmon/hwmon3/pwm{n}_enable"), "5\n");
            fs.set_file(format!("/sys/class/hwmon/hwmon3/fan{n}_input"), "0\n");
            fs.set_file(
                format!("/sys/class/hwmon/hwmon3/pwm{n}_auto_point1_temp"),
                "30000\n",
            );
        }
    }

    #[test]
    fn detect_empty_tree_returns_none() {
        let mut fs = FakeFs::default();
        fs.set_dir(PathBuf::from("/sys/class/hwmon"), vec![]);
        assert!(detect_chip(&fs, Path::new("/sys/class/hwmon")).is_none());
    }

    #[test]
    fn detect_unsupported_chip_returns_none() {
        let mut fs = FakeFs::default();
        fs.set_dir(
            PathBuf::from("/sys/class/hwmon"),
            vec![PathBuf::from("/sys/class/hwmon/hwmon0")],
        );
        fs.set_file("/sys/class/hwmon/hwmon0/name", "amdgpu\n");
        assert!(detect_chip(&fs, Path::new("/sys/class/hwmon")).is_none());
    }

    #[test]
    fn detect_nct6799_enumerates_pwm_channels() {
        let mut fs = FakeFs::default();
        nct6799_with_2_channels(&mut fs);
        let chip = detect_chip(&fs, Path::new("/sys/class/hwmon")).unwrap();
        assert_eq!(chip.name, "nct6799");
        assert_eq!(chip.path, PathBuf::from("/sys/class/hwmon/hwmon3"));
        let pwms: Vec<u8> = chip.channels.iter().map(|c| c.pwm).collect();
        assert_eq!(pwms, vec![2, 7]);
        for ch in &chip.channels {
            assert!(ch.has_enable);
            assert!(ch.has_smart_fan_iv);
        }
    }

    #[test]
    fn detect_prefix_match_accepts_nct_with_suffix() {
        let mut fs = FakeFs::default();
        fs.set_dir(
            PathBuf::from("/sys/class/hwmon"),
            vec![PathBuf::from("/sys/class/hwmon/hwmon3")],
        );
        fs.set_file("/sys/class/hwmon/hwmon3/name", "nct6799-isa-0290\n");
        fs.set_file("/sys/class/hwmon/hwmon3/pwm1", "0\n");
        fs.set_file("/sys/class/hwmon/hwmon3/pwm1_enable", "5\n");
        fs.set_file("/sys/class/hwmon/hwmon3/fan1_input", "0\n");
        let chip = detect_chip(&fs, Path::new("/sys/class/hwmon")).unwrap();
        assert!(chip.name.starts_with("nct6799"));
    }

    #[test]
    fn detect_prefers_chip_with_more_pwm_nodes() {
        let mut fs = FakeFs::default();
        fs.set_dir(
            PathBuf::from("/sys/class/hwmon"),
            vec![
                PathBuf::from("/sys/class/hwmon/hwmon0"),
                PathBuf::from("/sys/class/hwmon/hwmon1"),
            ],
        );
        // hwmon0 = nct6798 with 1 channel
        fs.set_file("/sys/class/hwmon/hwmon0/name", "nct6798\n");
        fs.set_file("/sys/class/hwmon/hwmon0/pwm1", "0\n");
        fs.set_file("/sys/class/hwmon/hwmon0/pwm1_enable", "5\n");
        fs.set_file("/sys/class/hwmon/hwmon0/fan1_input", "0\n");
        // hwmon1 = nct6799 with 2 channels — should win
        fs.set_file("/sys/class/hwmon/hwmon1/name", "nct6799\n");
        for n in [1u8, 2u8] {
            fs.set_file(format!("/sys/class/hwmon/hwmon1/pwm{n}"), "0\n");
            fs.set_file(format!("/sys/class/hwmon/hwmon1/pwm{n}_enable"), "5\n");
            fs.set_file(format!("/sys/class/hwmon/hwmon1/fan{n}_input"), "0\n");
        }
        let chip = detect_chip(&fs, Path::new("/sys/class/hwmon")).unwrap();
        assert_eq!(chip.name, "nct6799");
    }

    #[test]
    fn detect_skips_pwm_without_fan_input() {
        let mut fs = FakeFs::default();
        fs.set_dir(
            PathBuf::from("/sys/class/hwmon"),
            vec![PathBuf::from("/sys/class/hwmon/hwmon3")],
        );
        fs.set_file("/sys/class/hwmon/hwmon3/name", "nct6799\n");
        fs.set_file("/sys/class/hwmon/hwmon3/pwm1", "0\n");
        fs.set_file("/sys/class/hwmon/hwmon3/pwm1_enable", "5\n");
        fs.set_file("/sys/class/hwmon/hwmon3/fan1_input", "0\n");
        // pwm2 with no fan input — should be skipped
        fs.set_file("/sys/class/hwmon/hwmon3/pwm2", "0\n");
        let chip = detect_chip(&fs, Path::new("/sys/class/hwmon")).unwrap();
        let pwms: Vec<u8> = chip.channels.iter().map(|c| c.pwm).collect();
        assert_eq!(pwms, vec![1]);
    }

    #[test]
    fn detect_returns_none_when_supported_chip_has_no_pwm_nodes() {
        // asus_wmi_sensors chips expose sensor temps but no pwmN nodes.
        let mut fs = FakeFs::default();
        fs.set_dir(
            PathBuf::from("/sys/class/hwmon"),
            vec![PathBuf::from("/sys/class/hwmon/hwmon5")],
        );
        fs.set_file("/sys/class/hwmon/hwmon5/name", "asus_wmi_sensors\n");
        fs.set_file("/sys/class/hwmon/hwmon5/temp1_input", "45000\n");
        assert!(detect_chip(&fs, Path::new("/sys/class/hwmon")).is_none());
    }
}
