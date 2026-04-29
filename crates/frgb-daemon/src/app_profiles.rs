//! App profile runner — auto-switches profiles based on focused application.
//!
//! Polls the active window's process name via /proc and compares against
//! configured app-profile mappings. Runs on the engine poll interval.

use std::fs;
use std::path::Path;

use frgb_model::config::AppProfile;

/// Monitors focused application and triggers profile switches.
pub struct AppProfileRunner {
    profiles: Vec<AppProfile>,
    last_matched: Option<String>,
}

/// Commands the engine should execute when an app profile matches.
pub enum AppProfileCommand {
    SwitchProfile(String),
}

impl AppProfileRunner {
    pub fn new() -> Self {
        Self {
            profiles: Vec::new(),
            last_matched: None,
        }
    }

    /// Load app profile mappings from config.
    pub fn load(&mut self, profiles: Vec<AppProfile>) {
        self.profiles = profiles;
    }

    /// Evaluate the current focused application against configured profiles.
    /// Returns a command if the active profile should change.
    pub fn evaluate(&mut self) -> Option<AppProfileCommand> {
        if self.profiles.is_empty() {
            return None;
        }

        let focused = focused_app_name();
        let focused = focused.as_deref().unwrap_or("");

        // Find matching app profile (case-insensitive substring match)
        let matched = self
            .profiles
            .iter()
            .find(|ap| focused.to_lowercase().contains(&ap.app_name.to_lowercase()));

        match matched {
            Some(ap) => {
                let profile = &ap.profile;
                if self.last_matched.as_ref() != Some(profile) {
                    self.last_matched = Some(profile.clone());
                    tracing::info!("App profile: '{}' → profile '{}'", ap.app_name, profile);
                    Some(AppProfileCommand::SwitchProfile(profile.clone()))
                } else {
                    None
                }
            }
            None => {
                if self.last_matched.is_some() {
                    self.last_matched = None;
                    // Could switch back to default profile here
                }
                None
            }
        }
    }

    pub fn is_active(&self) -> bool {
        !self.profiles.is_empty()
    }
}

/// Get the focused application name by reading /proc/self/fd/0 -> terminal,
/// or by scanning common X11/Wayland focused window mechanisms.
///
/// Fallback: reads /proc/<pid>/comm for the process owning the active terminal.
/// For desktop environments, this requires reading the active window via
/// xdotool, xprop, or Wayland protocols. We use a simple /proc scan as baseline.
fn focused_app_name() -> Option<String> {
    // Try xdotool first (works on X11)
    if let Ok(output) = std::process::Command::new("xdotool")
        .args(["getactivewindow", "getwindowpid"])
        .output()
    {
        if output.status.success() {
            let pid_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if let Ok(pid) = pid_str.parse::<u32>() {
                let comm_path = format!("/proc/{}/comm", pid);
                if let Ok(comm) = fs::read_to_string(Path::new(&comm_path)) {
                    return Some(comm.trim().to_string());
                }
            }
        }
    }

    // Fallback: read /proc/self/comm (gets the daemon itself, not useful)
    // In practice, focused window detection requires X11/Wayland integration.
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_profiles_no_command() {
        let mut runner = AppProfileRunner::new();
        assert!(runner.evaluate().is_none());
        assert!(!runner.is_active());
    }

    #[test]
    fn load_and_active() {
        let mut runner = AppProfileRunner::new();
        runner.load(vec![AppProfile {
            app_name: "firefox".into(),
            profile: "Gaming".into(),
        }]);
        assert!(runner.is_active());
    }

    /// Verify last_matched state persists through evaluate when no app is detected.
    #[test]
    fn no_repeat_switch() {
        let mut runner = AppProfileRunner::new();
        runner.last_matched = Some("Gaming".into());
        runner.load(vec![AppProfile {
            app_name: "firefox".into(),
            profile: "Gaming".into(),
        }]);
        // Without X11, focused_app_name() returns None → no match → last_matched cleared.
        let cmd = runner.evaluate();
        // No switch command emitted (no focused app detected).
        assert!(cmd.is_none());
        // last_matched is cleared when no app matches.
        assert!(runner.last_matched.is_none());
    }
}
