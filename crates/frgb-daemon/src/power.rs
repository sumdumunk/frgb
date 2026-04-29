//! Power management runner — switches profiles based on AC/battery state.
//!
//! Reads power supply status from /sys/class/power_supply/ without D-Bus.
//! Supports AC adapter detection for automatic profile switching between
//! performance (AC) and quiet (battery) modes.

use std::fs;
use std::path::Path;

use frgb_model::config::PowerConfig;

/// Commands the engine should execute on power state change.
pub enum PowerCommand {
    SwitchProfile(String),
}

/// Monitors AC/battery state and triggers profile switches.
pub struct PowerRunner {
    config: Option<PowerConfig>,
    last_on_ac: Option<bool>,
}

impl PowerRunner {
    pub fn new() -> Self {
        Self {
            config: None,
            last_on_ac: None,
        }
    }

    pub fn set_config(&mut self, config: PowerConfig) {
        self.config = Some(config);
    }

    /// Check current power state and return a command if it changed.
    pub fn evaluate(&mut self) -> Option<PowerCommand> {
        let config = self.config.as_ref()?;
        let on_ac = is_on_ac_power();

        if self.last_on_ac == Some(on_ac) {
            return None;
        }
        self.last_on_ac = Some(on_ac);

        let profile = if on_ac {
            config.on_ac.as_ref()
        } else {
            config.on_battery.as_ref()
        };

        profile.map(|name| {
            tracing::info!("Power: {} → profile '{}'", if on_ac { "AC" } else { "battery" }, name);
            PowerCommand::SwitchProfile(name.clone())
        })
    }

    pub fn is_active(&self) -> bool {
        self.config.is_some()
    }

    /// Return the last observed AC power state (true = AC, false = battery).
    /// Falls back to true (desktop assumption) if never evaluated.
    pub fn is_on_ac(&self) -> bool {
        self.last_on_ac.unwrap_or(true)
    }
}

/// Check if the system is on AC power by reading /sys/class/power_supply/.
///
/// Returns true if any AC adapter reports "online", or if no battery is detected
/// (desktop systems are always "on AC").
fn is_on_ac_power() -> bool {
    let ps_dir = Path::new("/sys/class/power_supply");
    if !ps_dir.exists() {
        return true; // No power_supply sysfs = desktop, assume AC
    }

    let Ok(entries) = fs::read_dir(ps_dir) else { return true };

    let mut has_battery = false;

    for entry in entries.flatten() {
        let path = entry.path();
        let type_path = path.join("type");
        let Ok(supply_type) = fs::read_to_string(&type_path) else {
            continue;
        };
        let supply_type = supply_type.trim();

        if supply_type == "Mains" {
            // AC adapter — check "online" file
            let online_path = path.join("online");
            if let Ok(val) = fs::read_to_string(&online_path) {
                if val.trim() == "1" {
                    return true;
                }
            }
        } else if supply_type == "Battery" {
            has_battery = true;
        }
    }

    // No AC adapter found — if no battery exists, it's a desktop (AC)
    !has_battery
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_config_no_command() {
        let mut runner = PowerRunner::new();
        assert!(runner.evaluate().is_none());
        assert!(!runner.is_active());
    }

    #[test]
    fn is_on_ac_power_returns_bool() {
        // Just verify it doesn't panic — actual value depends on hardware
        let _result = is_on_ac_power();
    }
}
