use crate::error::Result;
use crate::hwmon_backend::counters;
use crate::hwmon_backend::detect::detect_chip;
use crate::hwmon_backend::fs::HwmonFs;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const COOLDOWN: Duration = Duration::from_secs(5);

/// Run `op(&chip_path)`; on failure, re-scan the hwmon base for a chip
/// matching `chip_name`. If the chip's path changed, update the cached
/// `chip_path` and retry `op` once. Cooldown prevents rescan storms when
/// the failure is persistent.
///
/// The closure is called with the CURRENT chip path each time, so the
/// retry sees the refreshed path automatically.
pub fn with_recovery<F, T, Op>(
    fs: &F,
    chip_name: &str,
    base: &Path,
    chip_path: &Cell<PathBuf>,
    cooldown: &Cell<Option<Instant>>,
    mut op: Op,
) -> Result<T>
where
    F: HwmonFs + ?Sized,
    Op: FnMut(&Path) -> Result<T>,
{
    let current = cell_clone(chip_path);
    let first = op(&current);
    match first {
        Ok(v) => Ok(v),
        Err(first_err) => {
            if chip_name.is_empty() {
                // No name to match against — a rescan cannot safely pick a
                // replacement chip, so bail out with the original error.
                return Err(first_err);
            }
            if let Some(last) = cooldown.get() {
                if last.elapsed() < COOLDOWN {
                    return Err(first_err);
                }
            }
            cooldown.set(Some(Instant::now()));
            counters::record_rescan_attempt();
            tracing::warn!("hwmon: op failed ({first_err}); re-scanning for chip '{chip_name}'");
            let Some(chip) = detect_chip(fs, base) else {
                return Err(first_err);
            };
            if !chip.name.starts_with(chip_name) && !chip_name.starts_with(&chip.name) {
                return Err(first_err);
            }
            let old_path = cell_clone(chip_path);
            if chip.path != old_path {
                tracing::info!(
                    "hwmon: chip path changed {} -> {}",
                    old_path.display(),
                    chip.path.display()
                );
                chip_path.set(chip.path.clone());
            }
            let retry_result = op(&chip.path);
            if retry_result.is_ok() {
                counters::record_rescan_success();
            }
            retry_result
        }
    }
}

/// Clone the contents of a `Cell<PathBuf>` without consuming them.
fn cell_clone(cell: &Cell<PathBuf>) -> PathBuf {
    let v = cell.take();
    let clone = v.clone();
    cell.set(v);
    clone
}

#[cfg(test)]
mod tests {
    use super::with_recovery;
    use crate::hwmon_backend::fs::tests_only::FakeFs;
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    fn fs_with_chip_at(hwmon_path: &str) -> FakeFs {
        let fs = FakeFs::default();
        fs.set_dir(
            PathBuf::from("/sys/class/hwmon"),
            vec![PathBuf::from(hwmon_path)],
        );
        fs.set_file(format!("{hwmon_path}/name"), "nct6799\n");
        fs.set_file(format!("{hwmon_path}/pwm1"), "0\n");
        fs.set_file(format!("{hwmon_path}/pwm1_enable"), "5\n");
        fs.set_file(format!("{hwmon_path}/fan1_input"), "0\n");
        fs
    }

    #[test]
    fn success_returns_immediately() {
        let fs = fs_with_chip_at("/sys/class/hwmon/hwmon3");
        let chip_path = Cell::new(PathBuf::from("/sys/class/hwmon/hwmon3"));
        let cooldown = Cell::new(None);
        let attempts = Cell::new(0);
        let r: crate::error::Result<()> = with_recovery(
            &fs,
            "nct6799",
            Path::new("/sys/class/hwmon"),
            &chip_path,
            &cooldown,
            |_| {
                attempts.set(attempts.get() + 1);
                Ok(())
            },
        );
        assert!(r.is_ok());
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn failure_rescans_and_retries_once() {
        let fs = fs_with_chip_at("/sys/class/hwmon/hwmon4"); // renumbered
        let chip_path = Cell::new(PathBuf::from("/sys/class/hwmon/hwmon3")); // stale
        let cooldown = Cell::new(None);
        let attempts = Cell::new(0);

        let _: crate::error::Result<()> = with_recovery(
            &fs,
            "nct6799",
            Path::new("/sys/class/hwmon"),
            &chip_path,
            &cooldown,
            |path| {
                attempts.set(attempts.get() + 1);
                if attempts.get() == 1 {
                    // First call fails — simulating write to stale path
                    Err(crate::error::CoreError::Protocol("stale".into()))
                } else {
                    // Second call should see refreshed path
                    assert_eq!(path, Path::new("/sys/class/hwmon/hwmon4"));
                    Ok(())
                }
            },
        );
        assert_eq!(attempts.get(), 2);
        // Cached path was updated
        let p = chip_path.take();
        assert_eq!(p, PathBuf::from("/sys/class/hwmon/hwmon4"));
    }

    #[test]
    fn cooldown_blocks_second_rescan() {
        let fs = fs_with_chip_at("/sys/class/hwmon/hwmon3");
        let chip_path = Cell::new(PathBuf::from("/sys/class/hwmon/hwmon3"));
        // Recent cooldown — should prevent re-scan
        let cooldown = Cell::new(Some(Instant::now()));
        let attempts = Cell::new(0);

        let r: crate::error::Result<()> = with_recovery(
            &fs,
            "nct6799",
            Path::new("/sys/class/hwmon"),
            &chip_path,
            &cooldown,
            |_| {
                attempts.set(attempts.get() + 1);
                Err(crate::error::CoreError::Protocol("x".into()))
            },
        );
        assert!(r.is_err());
        assert_eq!(attempts.get(), 1); // no retry
    }

    #[test]
    fn rescan_failure_returns_original_error() {
        let fs = FakeFs::default();
        fs.set_dir(PathBuf::from("/sys/class/hwmon"), vec![]); // no chips
        let chip_path = Cell::new(PathBuf::from("/sys/class/hwmon/hwmon3"));
        let cooldown = Cell::new(None);

        let r: crate::error::Result<()> = with_recovery(
            &fs,
            "nct6799",
            Path::new("/sys/class/hwmon"),
            &chip_path,
            &cooldown,
            |_| Err(crate::error::CoreError::Protocol("first".into())),
        );
        // Original error returned (rescan found nothing to retry against).
        assert!(matches!(r, Err(crate::error::CoreError::Protocol(_))));
    }

    #[test]
    fn rescan_finds_wrong_chip_returns_original_error() {
        let fs = FakeFs::default();
        fs.set_dir(
            PathBuf::from("/sys/class/hwmon"),
            vec![PathBuf::from("/sys/class/hwmon/hwmon3")],
        );
        // Supported (nct6799) but cached name is it87 — should NOT match
        fs.set_file("/sys/class/hwmon/hwmon3/name", "nct6799\n");
        fs.set_file("/sys/class/hwmon/hwmon3/pwm1", "0\n");
        fs.set_file("/sys/class/hwmon/hwmon3/pwm1_enable", "5\n");
        fs.set_file("/sys/class/hwmon/hwmon3/fan1_input", "0\n");

        let chip_path = Cell::new(PathBuf::from("/sys/class/hwmon/hwmon3"));
        let cooldown = Cell::new(None);
        let attempts = Cell::new(0);

        let r: crate::error::Result<()> = with_recovery(
            &fs,
            "it87", // cached name differs
            Path::new("/sys/class/hwmon"),
            &chip_path,
            &cooldown,
            |_| {
                attempts.set(attempts.get() + 1);
                Err(crate::error::CoreError::Protocol("first".into()))
            },
        );
        assert!(r.is_err());
        // Only one op attempt — no retry because name didn't match.
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn empty_chip_name_bypasses_rescan() {
        let fs = fs_with_chip_at("/sys/class/hwmon/hwmon3");
        let chip_path = Cell::new(PathBuf::from("/sys/class/hwmon/hwmon3"));
        let cooldown = Cell::new(None);
        let attempts = Cell::new(0);

        let r: crate::error::Result<()> = with_recovery(
            &fs,
            "", // empty name
            Path::new("/sys/class/hwmon"),
            &chip_path,
            &cooldown,
            |_| {
                attempts.set(attempts.get() + 1);
                Err(crate::error::CoreError::Protocol("first".into()))
            },
        );
        assert!(r.is_err());
        // Only one op attempt — no retry because empty name cannot match.
        assert_eq!(attempts.get(), 1);
    }
}
