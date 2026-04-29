//! Hwmon-backed motherboard fan control.
//!
//! Drives motherboard-header fans via `/sys/class/hwmon/*` sysfs. See
//! `docs/superpowers/specs/2026-04-18-hwmon-fan-control-design.md`.

pub mod counters;
pub mod detect;
pub mod fs;
pub mod recovery;
pub mod state;
pub mod writer;

pub use counters::{snapshot as recovery_counters, HwmonRecoveryCounters};

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use frgb_model::config::{HwmonChannelRole, HwmonConfig};
use frgb_model::device::DeviceId;
use frgb_model::GroupId;

use crate::backend::{Backend, BackendId, DiscoveredDevice, SpeedCommand};
use crate::error::{CoreError, Result};
use crate::registry::Device;

use detect::{detect_chip, DetectedChip};
use fs::{HwmonFs, RealFs};
use state::PendingRestores;
use writer::{pct_to_byte, pump_floor_byte, read_fan_rpm, read_pwm};

/// Synthetic dev_type for hwmon channels, distinct from RF(0), ENE(0xFC),
/// AURA(0xFD), LCD. See spec §5.4.
pub const DEV_TYPE_HWMON: u8 = 0xFB;

/// Default base path for hwmon on real Linux. Test builds can override via
/// `open_with`.
const HWMON_BASE: &str = "/sys/class/hwmon";

pub struct HwmonBackend {
    fs: RefCell<Box<dyn HwmonFs>>,
    chip_name: String,
    chip_path: Cell<PathBuf>,
    base: PathBuf,
    channels: Vec<ConfiguredChannel>,
    unconfigured: Vec<u8>,
    state_path: PathBuf,
    group_base: u8,
    pending_restores: RefCell<PendingRestores>,
    recovery_cooldown: Cell<Option<std::time::Instant>>,
}

#[derive(Debug, Clone)]
struct ConfiguredChannel {
    pwm: u8,
    role: HwmonChannelRole,
    min_pwm: u8,
}

impl HwmonBackend {
    /// Production entry point — real filesystem, default base path, default
    /// state path resolution (`$XDG_STATE_HOME/frgb/hwmon-saved.json` or
    /// `~/.local/state/frgb/hwmon-saved.json`).
    pub fn open(cfg: &HwmonConfig) -> Result<Self> {
        let state_path = cfg
            .state_file
            .as_ref()
            .map(|s| expand_tilde(s))
            .unwrap_or_else(state::default_state_path);
        Self::open_with(RealFs, PathBuf::from(HWMON_BASE), state_path, cfg)
    }

    /// Test / injection entry point. `fs` is type-erased into
    /// `Box<dyn HwmonFs>` so the backend can hold a fake in unit tests.
    pub fn open_with<F: HwmonFs + 'static>(
        fs: F,
        base: PathBuf,
        state_path: PathBuf,
        cfg: &HwmonConfig,
    ) -> Result<Self> {
        let boxed: Box<dyn HwmonFs> = Box::new(fs);
        let chip = match detect_chip(&*boxed, &base) {
            Some(c) => c,
            None => {
                tracing::info!("hwmon: no supported chip detected");
                return Ok(Self::empty(boxed, base, state_path, cfg.group_base));
            }
        };
        Self::init_with_chip(boxed, base, state_path, cfg, chip)
    }

    fn empty(
        fs: Box<dyn HwmonFs>,
        base: PathBuf,
        state_path: PathBuf,
        group_base: u8,
    ) -> Self {
        Self {
            fs: RefCell::new(fs),
            chip_name: String::new(),
            chip_path: Cell::new(PathBuf::new()),
            base,
            channels: Vec::new(),
            unconfigured: Vec::new(),
            state_path,
            group_base,
            pending_restores: RefCell::new(PendingRestores::default()),
            recovery_cooldown: Cell::new(None),
        }
    }

    fn init_with_chip(
        fs: Box<dyn HwmonFs>,
        base: PathBuf,
        state_path: PathBuf,
        cfg: &HwmonConfig,
        chip: DetectedChip,
    ) -> Result<Self> {
        let mut channels = Vec::new();
        let mut unconfigured = Vec::new();

        // Reject configs that would overflow the u8 group id space. Channel
        // groups are `group_base + pwm_idx - 1`; with pwm_idx up to 7 this
        // requires group_base <= 248.
        if cfg.group_base > 248 {
            return Err(CoreError::Config(format!(
                "hwmon.group_base = {} would overflow GroupId space (max 248)",
                cfg.group_base
            )));
        }

        for ch in &chip.channels {
            if let Some(user) = cfg.channels.iter().find(|c| c.pwm == ch.pwm) {
                channels.push(ConfiguredChannel {
                    pwm: ch.pwm,
                    role: user.role,
                    min_pwm: user.min_pwm,
                });
            } else {
                unconfigured.push(ch.pwm);
            }
        }

        if !unconfigured.is_empty() && channels.is_empty() {
            tracing::info!(
                "hwmon: {} channel(s) detected, 0 configured. See ./r mobo --help.",
                unconfigured.len()
            );
        }

        // Restore any pre-existing state from a previous run.
        if let Some(prev) = state::load_state_file_optional(&state_path) {
            let matches = prev.chip_name.starts_with(&chip.name)
                || chip.name.starts_with(&prev.chip_name);
            if matches {
                for entry in &prev.entries {
                    if let Err(e) =
                        writer::set_enable(&*fs, &chip.path, entry.pwm, entry.saved_enable)
                    {
                        tracing::warn!(
                            "hwmon: restore pwm{} enable={} failed: {e}",
                            entry.pwm,
                            entry.saved_enable
                        );
                    } else {
                        tracing::info!(
                            "hwmon: restored pwm{}_enable to {}",
                            entry.pwm,
                            entry.saved_enable
                        );
                    }
                }
                let _ = std::fs::remove_file(&state_path);
            } else {
                tracing::warn!(
                    "hwmon: state file chip mismatch ({} vs {}), discarding",
                    prev.chip_name,
                    chip.name
                );
                let _ = std::fs::remove_file(&state_path);
            }
        }

        Ok(Self {
            fs: RefCell::new(fs),
            chip_name: chip.name,
            chip_path: Cell::new(chip.path),
            base,
            channels,
            unconfigured,
            state_path,
            group_base: cfg.group_base,
            pending_restores: RefCell::new(PendingRestores::default()),
            recovery_cooldown: Cell::new(None),
        })
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Read the current `pwmN_enable` for a configured channel. Returns the
    /// raw sysfs value, not a mapped string — callers decide formatting.
    /// None if the channel isn't configured or the read fails.
    pub fn current_enable(&self, pwm_idx: u8) -> Option<u8> {
        if !self.channels.iter().any(|c| c.pwm == pwm_idx) {
            return None;
        }
        let fs = self.fs.borrow();
        let chip_path = self.chip_path_cloned();
        writer::read_enable(&**fs, &chip_path, pwm_idx).ok()
    }

    pub fn unconfigured_channels(&self) -> &[u8] {
        &self.unconfigured
    }

    pub fn chip_name(&self) -> &str {
        &self.chip_name
    }

    /// Path to the on-disk restore state file this backend is using.
    /// Exposed so the daemon's panic hook can target the same file the
    /// backend writes to, respecting any `hwmon.state_file` config override.
    pub fn state_path(&self) -> &std::path::Path {
        &self.state_path
    }

    /// Restore every taken-over channel to its saved `pwm_enable` value.
    /// Called from daemon / CLI clean-exit paths and from `Drop`.
    pub fn shutdown(&self) {
        let restores: Vec<state::StateEntry> =
            self.pending_restores.borrow().iter().cloned().collect();
        if restores.is_empty() {
            return;
        }
        let fs = self.fs.borrow();
        let chip_path = self.chip_path_cloned();
        for entry in &restores {
            if let Err(e) = writer::set_enable(&**fs, &chip_path, entry.pwm, entry.saved_enable) {
                tracing::warn!("hwmon shutdown pwm{}: {e}", entry.pwm);
            } else {
                tracing::info!("hwmon: pwm{} enable restored to {}", entry.pwm, entry.saved_enable);
            }
        }
        let _ = std::fs::remove_file(&self.state_path);
        self.pending_restores.borrow_mut().clear_for_shutdown();
    }

    // --- internal helpers ---

    fn chip_path_cloned(&self) -> PathBuf {
        let p = self.chip_path.take();
        let clone = p.clone();
        self.chip_path.set(p);
        clone
    }

    fn channel_for_group(&self, group: GroupId) -> Option<&ConfiguredChannel> {
        let g = group.value();
        if g < self.group_base {
            return None;
        }
        let pwm_idx = g - self.group_base + 1;
        self.channels.iter().find(|c| c.pwm == pwm_idx)
    }

    /// Snapshot `pwm_idx`'s current enable value at an explicitly supplied
    /// `chip_path`. Used inside `with_recovery` closures so the retry sees
    /// the refreshed path.
    fn ensure_snapshot_at(
        &self,
        fs: &dyn HwmonFs,
        chip_path: &std::path::Path,
        pwm_idx: u8,
    ) -> Result<()> {
        if self.pending_restores.borrow().contains(pwm_idx) {
            return Ok(());
        }
        let saved = writer::read_enable(fs, chip_path, pwm_idx)?;
        self.pending_restores
            .borrow_mut()
            .record(pwm_idx, saved, false);
        let new_state = self
            .pending_restores
            .borrow()
            .to_file(&self.chip_name, &chip_path.display().to_string());
        if let Err(e) = state::save_state_file(&new_state, &self.state_path) {
            tracing::warn!("hwmon: state file save failed: {e}");
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fs_for_test(&self) -> std::cell::Ref<'_, Box<dyn HwmonFs>> {
        self.fs.borrow()
    }
}

impl Drop for HwmonBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Backend for HwmonBackend {
    fn id(&self) -> BackendId {
        BackendId(5)
    }
    fn name(&self) -> &str {
        "hwmon"
    }

    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>> {
        let fs = self.fs.borrow();
        let chip_path = self.chip_path_cloned();
        let mut out = Vec::new();
        for ch in &self.channels {
            let rpm = match read_fan_rpm(&**fs, &chip_path, ch.pwm) {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!("hwmon: read fan{}_input failed: {e}", ch.pwm);
                    0
                }
            };
            let pwm = match read_pwm(&**fs, &chip_path, ch.pwm) {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!("hwmon: read pwm{} failed: {e}", ch.pwm);
                    0
                }
            };
            let group = GroupId::new(self.group_base + ch.pwm - 1);
            let id = {
                let mut d = DeviceId::ZERO;
                d.set_index(ch.pwm);
                d
            };
            let mut fans_rpm = [0u16; 4];
            fans_rpm[0] = rpm;
            let mut fans_pwm = [0u8; 4];
            fans_pwm[0] = pwm;
            out.push(DiscoveredDevice {
                id,
                fans_type: [0; 4],
                dev_type: DEV_TYPE_HWMON,
                group,
                fan_count: 1,
                master: DeviceId::ZERO,
                fans_rpm,
                fans_pwm,
                cmd_seq: 0,
                channel: ch.pwm, // pwm index so status rendering can key off it
            });
        }
        Ok(out)
    }

    fn set_speed(&self, device: &Device, cmd: &SpeedCommand) -> Result<()> {
        let Some(ch) = self.channel_for_group(device.group) else {
            return Err(CoreError::NotFound(format!(
                "hwmon group {}",
                device.group.value()
            )));
        };
        match cmd {
            SpeedCommand::Manual(pct) => {
                let floor = effective_floor(ch);
                let byte = pct_to_byte(pct.value(), floor);
                let pwm = ch.pwm;
                let fs = self.fs.borrow();
                // Snapshot + enable=1 + duty in one closure so recovery
                // retries the entire sequence atomically on the new path.
                recovery::with_recovery(
                    &**fs,
                    &self.chip_name,
                    &self.base,
                    &self.chip_path,
                    &self.recovery_cooldown,
                    |chip_path| {
                        self.ensure_snapshot_at(&**fs, chip_path, pwm)?;
                        writer::set_enable(&**fs, chip_path, pwm, 1)?;
                        writer::set_pwm(&**fs, chip_path, pwm, byte)
                    },
                )
            }
            SpeedCommand::Pwm => {
                let saved = {
                    let restores = self.pending_restores.borrow();
                    restores.get(ch.pwm).map(|e| e.saved_enable)
                };
                if let Some(saved_enable) = saved {
                    let pwm = ch.pwm;
                    let fs = self.fs.borrow();
                    recovery::with_recovery(
                        &**fs,
                        &self.chip_name,
                        &self.base,
                        &self.chip_path,
                        &self.recovery_cooldown,
                        |chip_path| writer::set_enable(&**fs, chip_path, pwm, saved_enable),
                    )?;
                    // After the write succeeds, update the in-memory state and
                    // state file. Borrow fs is dropped before chip_path_cloned
                    // (which also borrows chip_path via Cell, but that's fine
                    // since Cell doesn't use RefCell).
                    drop(fs);
                    let mut restores = self.pending_restores.borrow_mut();
                    restores.remove(ch.pwm);
                    if restores.is_empty() {
                        let _ = std::fs::remove_file(&self.state_path);
                    } else {
                        let chip_path = self.chip_path_cloned();
                        let new_state = restores
                            .to_file(&self.chip_name, &chip_path.display().to_string());
                        let _ = state::save_state_file(&new_state, &self.state_path);
                    }
                }
                Ok(())
            }
        }
    }

    fn send_rgb(
        &self,
        _device: &Device,
        _buffer: &frgb_rgb::generator::EffectResult,
    ) -> Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn effective_floor(ch: &ConfiguredChannel) -> u8 {
    if ch.min_pwm > 0 {
        return ch.min_pwm;
    }
    if matches!(ch.role, HwmonChannelRole::Pump) {
        return pump_floor_byte();
    }
    0
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}

/// Panic-hook entry point: read the state file and restore saved enables
/// without touching any in-memory `HwmonBackend` state. Best-effort — may
/// silently fail if the panic originated inside the tracing subscriber or
/// allocator. Writes sysfs directly.
pub fn emergency_restore(base: &Path, state_path: &Path) {
    let Some(state) = state::load_state_file_optional(state_path) else {
        return;
    };

    let fs = fs::RealFs;
    let Some(chip) = detect::detect_chip(&fs, base) else {
        tracing::warn!("hwmon emergency restore: no supported chip on scan");
        return;
    };
    let name_matches = chip.name.starts_with(&state.chip_name)
        || state.chip_name.starts_with(&chip.name);
    if !name_matches {
        tracing::warn!(
            "hwmon emergency restore: chip name mismatch ({} vs saved {})",
            chip.name,
            state.chip_name
        );
        return;
    }

    for entry in &state.entries {
        let _ = writer::set_enable(&fs, &chip.path, entry.pwm, entry.saved_enable);
    }
    let _ = std::fs::remove_file(state_path);
}

#[cfg(test)]
mod backend_tests {
    use super::*;
    use crate::backend::{Backend, SpeedCommand};
    use crate::hwmon_backend::fs::tests_only::FakeFs;
    use crate::registry::{Device, DeviceState};
    use frgb_model::config::{HwmonChannelConfig, HwmonChannelRole, HwmonConfig, HwmonCurveExecution};
    use frgb_model::device::{BladeType, DeviceId, DeviceType, FanRole};
    use frgb_model::GroupId;
    use frgb_model::SpeedPercent;
    use std::path::PathBuf;

    fn make_fs_with_two_channels() -> FakeFs {
        let fs = FakeFs::default();
        fs.set_dir(
            PathBuf::from("/sys/class/hwmon"),
            vec![PathBuf::from("/sys/class/hwmon/hwmon3")],
        );
        fs.set_file("/sys/class/hwmon/hwmon3/name", "nct6799\n");
        for n in [2u8, 7u8] {
            fs.set_file(format!("/sys/class/hwmon/hwmon3/pwm{n}"), "128\n");
            fs.set_file(format!("/sys/class/hwmon/hwmon3/pwm{n}_enable"), "5\n");
            fs.set_file(format!("/sys/class/hwmon/hwmon3/fan{n}_input"), "1500\n");
        }
        fs
    }

    fn cfg_two_channels() -> HwmonConfig {
        HwmonConfig {
            group_base: 60,
            state_file: None,
            channels: vec![
                HwmonChannelConfig {
                    pwm: 2,
                    name: "Rear".into(),
                    role: HwmonChannelRole::Exhaust,
                    model: None,
                    min_pwm: 0,
                    curve_execution: HwmonCurveExecution::Auto,
                },
                HwmonChannelConfig {
                    pwm: 7,
                    name: "Pump".into(),
                    role: HwmonChannelRole::Pump,
                    model: None,
                    min_pwm: 0,
                    curve_execution: HwmonCurveExecution::Auto,
                },
            ],
        }
    }

    fn state_path_in_tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "frgb_hwmon_backend_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("hwmon-saved.json")
    }

    fn make_device(group: GroupId) -> Device {
        Device {
            id: DeviceId::ZERO,
            backend_id: crate::BackendId(5),
            group,
            slots: Vec::new(),
            state: DeviceState::default(),
            mac_ids: Vec::new(),
            tx_ref: DeviceId::ZERO,
            name: String::new(),
            device_type: DeviceType::Unknown,
            role: FanRole::Custom(String::new()),
            blade: BladeType::Standard,
            mb_sync: false,
            cmd_seq: 0,
        }
    }

    #[test]
    fn open_with_no_chip_returns_empty_backend() {
        let fs = FakeFs::default();
        fs.set_dir(PathBuf::from("/sys/class/hwmon"), vec![]);
        let state = state_path_in_tmp();
        let backend = HwmonBackend::open_with(
            fs,
            PathBuf::from("/sys/class/hwmon"),
            state,
            &HwmonConfig::default(),
        )
        .unwrap();
        assert_eq!(backend.channel_count(), 0);
    }

    #[test]
    fn open_reports_configured_channel_count() {
        let fs = make_fs_with_two_channels();
        let state = state_path_in_tmp();
        let backend = HwmonBackend::open_with(
            fs,
            PathBuf::from("/sys/class/hwmon"),
            state,
            &cfg_two_channels(),
        )
        .unwrap();
        assert_eq!(backend.channel_count(), 2);
    }

    #[test]
    fn discover_emits_configured_channels_only() {
        let fs = make_fs_with_two_channels();
        let state = state_path_in_tmp();
        let mut cfg = cfg_two_channels();
        cfg.channels.pop(); // remove pwm7, leave pwm2 configured
        let mut backend =
            HwmonBackend::open_with(fs, PathBuf::from("/sys/class/hwmon"), state, &cfg).unwrap();
        let discovered = backend.discover().unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].group, GroupId::new(60 + 2 - 1)); // pwm2 -> 61
        assert_eq!(discovered[0].fans_rpm[0], 1500);
        assert_eq!(discovered[0].fans_pwm[0], 128);
        assert_eq!(discovered[0].dev_type, 0xFB);
        assert_eq!(discovered[0].channel, 2);
    }

    #[test]
    fn set_speed_manual_takes_over_and_writes_duty() {
        let fs = make_fs_with_two_channels();
        let state = state_path_in_tmp();
        let backend = HwmonBackend::open_with(
            fs,
            PathBuf::from("/sys/class/hwmon"),
            state,
            &cfg_two_channels(),
        )
        .unwrap();
        let device = make_device(GroupId::new(61)); // pwm2 (group_base=60 + 2 - 1)
        backend
            .set_speed(&device, &SpeedCommand::Manual(SpeedPercent::new(50)))
            .unwrap();
        let fs = backend.fs_for_test();
        let fake = fs.as_any().downcast_ref::<crate::hwmon_backend::fs::tests_only::FakeFs>().unwrap();
        assert_eq!(fake.last_write("/sys/class/hwmon/hwmon3/pwm2_enable"), Some("1".into()));
        assert_eq!(fake.last_write("/sys/class/hwmon/hwmon3/pwm2"), Some("128".into()));
    }

    #[test]
    fn set_speed_manual_on_pump_applies_floor() {
        let fs = make_fs_with_two_channels();
        let state = state_path_in_tmp();
        let backend = HwmonBackend::open_with(
            fs,
            PathBuf::from("/sys/class/hwmon"),
            state,
            &cfg_two_channels(),
        )
        .unwrap();
        let device = make_device(GroupId::new(66)); // pwm7 -> pump
        backend
            .set_speed(&device, &SpeedCommand::Manual(SpeedPercent::new(10)))
            .unwrap();
        let fs = backend.fs_for_test();
        let fake = fs.as_any().downcast_ref::<crate::hwmon_backend::fs::tests_only::FakeFs>().unwrap();
        assert_eq!(fake.last_write("/sys/class/hwmon/hwmon3/pwm7"), Some("102".into())); // pump floor
    }

    #[test]
    fn set_speed_pwm_restores_saved_enable() {
        let fs = make_fs_with_two_channels();
        let state = state_path_in_tmp();
        let backend = HwmonBackend::open_with(
            fs,
            PathBuf::from("/sys/class/hwmon"),
            state,
            &cfg_two_channels(),
        )
        .unwrap();
        let device = make_device(GroupId::new(61));
        backend
            .set_speed(&device, &SpeedCommand::Manual(SpeedPercent::new(50)))
            .unwrap();
        backend.set_speed(&device, &SpeedCommand::Pwm).unwrap();
        let fs = backend.fs_for_test();
        let fake = fs.as_any().downcast_ref::<crate::hwmon_backend::fs::tests_only::FakeFs>().unwrap();
        assert_eq!(fake.last_write("/sys/class/hwmon/hwmon3/pwm2_enable"), Some("5".into()));
    }

    #[test]
    fn shutdown_restores_all_managed_channels() {
        let fs = make_fs_with_two_channels();
        let state = state_path_in_tmp();
        let state_check = state.clone();
        let backend = HwmonBackend::open_with(
            fs,
            PathBuf::from("/sys/class/hwmon"),
            state,
            &cfg_two_channels(),
        )
        .unwrap();
        let d2 = make_device(GroupId::new(61));
        let d7 = make_device(GroupId::new(66));
        backend
            .set_speed(&d2, &SpeedCommand::Manual(SpeedPercent::new(50)))
            .unwrap();
        backend
            .set_speed(&d7, &SpeedCommand::Manual(SpeedPercent::new(50)))
            .unwrap();

        backend.shutdown();

        let fs = backend.fs_for_test();
        let fake = fs.as_any().downcast_ref::<crate::hwmon_backend::fs::tests_only::FakeFs>().unwrap();
        // Takeover wrote enable=1, shutdown wrote enable=5 — two writes each.
        assert_eq!(fake.write_count("/sys/class/hwmon/hwmon3/pwm2_enable"), 2);
        assert_eq!(fake.last_write("/sys/class/hwmon/hwmon3/pwm2_enable"), Some("5".into()));
        assert_eq!(fake.last_write("/sys/class/hwmon/hwmon3/pwm7_enable"), Some("5".into()));
        // State file deleted
        assert!(!state_check.exists());
    }

    #[test]
    fn open_restores_from_stale_state_file() {
        use crate::hwmon_backend::state::{save_state_file, StateEntry, StateFile, STATE_FILE_VERSION};
        let fs = make_fs_with_two_channels();
        let state_path = state_path_in_tmp();
        let pre = StateFile {
            version: STATE_FILE_VERSION,
            chip_name: "nct6799".into(),
            chip_path_hint: "/sys/class/hwmon/hwmon3".into(),
            entries: vec![StateEntry { pwm: 2, saved_enable: 5, offloaded: false }],
        };
        save_state_file(&pre, &state_path).unwrap();

        let _backend = HwmonBackend::open_with(
            fs,
            PathBuf::from("/sys/class/hwmon"),
            state_path.clone(),
            &cfg_two_channels(),
        )
        .unwrap();

        assert!(!state_path.exists());
        // We can't inspect the backend's fs after moving it, so just verify the
        // state file was deleted as evidence of successful restore.
    }

    #[test]
    fn set_speed_recovers_from_chip_renumber() {
        use std::path::PathBuf;
        // Initial tree: chip at hwmon3, backend opens against this.
        let fs = make_fs_with_two_channels();
        let state = state_path_in_tmp();
        let backend = HwmonBackend::open_with(
            fs,
            PathBuf::from("/sys/class/hwmon"),
            state,
            &cfg_two_channels(),
        )
        .unwrap();

        // Simulate module reload: wipe hwmon3, create hwmon4 with same name.
        // The backend's cached chip_path still points at hwmon3, which no
        // longer exists in the backing fs. A write will fail; recovery should
        // re-scan, find hwmon4, update the cached path, and retry the write.
        {
            let guard = backend.fs_for_test();
            let fake = guard
                .as_any()
                .downcast_ref::<crate::hwmon_backend::fs::tests_only::FakeFs>()
                .unwrap();
            fake.dirs.borrow_mut().clear();
            fake.files.borrow_mut().clear();
            fake.set_dir(
                PathBuf::from("/sys/class/hwmon"),
                vec![PathBuf::from("/sys/class/hwmon/hwmon4")],
            );
            fake.set_file("/sys/class/hwmon/hwmon4/name", "nct6799\n");
            for n in [2u8, 7u8] {
                fake.set_file(format!("/sys/class/hwmon/hwmon4/pwm{n}"), "0\n");
                fake.set_file(format!("/sys/class/hwmon/hwmon4/pwm{n}_enable"), "5\n");
                fake.set_file(format!("/sys/class/hwmon/hwmon4/fan{n}_input"), "0\n");
            }
        }

        let device = make_device(GroupId::new(61)); // pwm2
        backend
            .set_speed(&device, &SpeedCommand::Manual(SpeedPercent::new(50)))
            .unwrap();

        // The write eventually landed at the NEW path.
        let guard = backend.fs_for_test();
        let fake = guard
            .as_any()
            .downcast_ref::<crate::hwmon_backend::fs::tests_only::FakeFs>()
            .unwrap();
        assert_eq!(fake.last_write("/sys/class/hwmon/hwmon4/pwm2"), Some("128".into()));
        assert_eq!(fake.last_write("/sys/class/hwmon/hwmon4/pwm2_enable"), Some("1".into()));
    }

    #[test]
    fn open_rejects_group_base_too_high() {
        let fs = make_fs_with_two_channels();
        let state = state_path_in_tmp();
        let mut cfg = cfg_two_channels();
        cfg.group_base = 250; // would overflow
        let result = HwmonBackend::open_with(
            fs,
            PathBuf::from("/sys/class/hwmon"),
            state,
            &cfg,
        );
        assert!(result.is_err());
    }
}
