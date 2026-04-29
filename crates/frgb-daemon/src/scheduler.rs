//! Schedule runner — evaluates time-based schedule entries.
//!
//! Checks configured schedules against the current time/day and executes
//! matching actions. Runs on a 60-second check interval to avoid redundant
//! checks while ensuring schedules fire within a minute of their target time.

use std::time::Instant;

use frgb_model::config::{ScheduleAction, ScheduleEntry, Weekday};
use frgb_model::GroupId;

/// Actions the schedule runner wants the engine to execute.
#[derive(Debug, Clone)]
pub enum ScheduleCommand {
    SwitchProfile(String),
    SetSpeed {
        group: GroupId,
        percent: frgb_model::SpeedPercent,
    },
    ApplyCurve {
        group: GroupId,
        curve: String,
    },
}

/// Evaluates schedules against system clock.
pub struct ScheduleRunner {
    entries: Vec<ScheduleEntry>,
    /// Last minute we evaluated (hour * 60 + minute) to prevent repeat firing.
    last_evaluated_minute: Option<u16>,
    /// Last evaluation timestamp (enforce ~60s between checks).
    last_check: Instant,
}

impl ScheduleRunner {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            last_evaluated_minute: None,
            last_check: Instant::now(),
        }
    }

    /// Load schedule entries from config.
    pub fn load(&mut self, entries: Vec<ScheduleEntry>) {
        self.entries = entries;
    }

    /// Add a single schedule entry.
    pub fn add(&mut self, entry: ScheduleEntry) {
        self.entries.push(entry);
    }

    /// Remove a schedule entry by index.
    pub fn remove(&mut self, index: usize) -> bool {
        if index < self.entries.len() {
            self.entries.remove(index);
            true
        } else {
            false
        }
    }

    /// Clear all schedule entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Get all schedule entries (for listing).
    pub fn entries(&self) -> &[ScheduleEntry] {
        &self.entries
    }

    /// Evaluate schedules against the current time.
    /// Only checks once per minute to avoid repeat firing.
    /// Returns commands for schedules that match right now.
    pub fn evaluate(&mut self, all_groups: &[GroupId]) -> Vec<ScheduleCommand> {
        // Rate-limit to once per ~60 seconds
        if self.last_check.elapsed().as_secs() < 55 {
            return Vec::new();
        }
        self.last_check = Instant::now();

        let now = chrono_free_now();
        let current_minute = now.hour as u16 * 60 + now.minute as u16;

        // Already evaluated this minute? Skip.
        if self.last_evaluated_minute == Some(current_minute) {
            return Vec::new();
        }
        self.last_evaluated_minute = Some(current_minute);

        let mut commands = Vec::new();

        for entry in &self.entries {
            if entry.hour == now.hour
                && entry.minute == now.minute
                && (entry.days.is_empty() || entry.days.contains(&now.weekday))
            {
                tracing::info!(
                    "Schedule triggered: {:02}:{:02} {:?} → {:?}",
                    entry.hour,
                    entry.minute,
                    entry.days,
                    entry.action
                );
                match &entry.action {
                    ScheduleAction::SwitchProfile(name) => {
                        commands.push(ScheduleCommand::SwitchProfile(name.clone()));
                    }
                    ScheduleAction::SetSpeed { target, percent } => {
                        let groups = resolve_target(target, all_groups);
                        for g in groups {
                            commands.push(ScheduleCommand::SetSpeed {
                                group: g,
                                percent: *percent,
                            });
                        }
                    }
                    ScheduleAction::ApplyCurve { target, curve } => {
                        let groups = resolve_target(target, all_groups);
                        for g in groups {
                            commands.push(ScheduleCommand::ApplyCurve {
                                group: g,
                                curve: curve.clone(),
                            });
                        }
                    }
                }
            }
        }

        commands
    }

    /// Whether any schedules are configured.
    pub fn is_active(&self) -> bool {
        !self.entries.is_empty()
    }
}

fn resolve_target(target: &frgb_model::ipc::Target, all_groups: &[GroupId]) -> Vec<GroupId> {
    use frgb_model::ipc::Target;
    match target {
        Target::All => all_groups.to_vec(),
        Target::Group(g) => vec![*g],
        Target::Groups(gs) => gs.clone(),
        Target::Role(_) => all_groups.to_vec(), // role filtering needs system context
    }
}

/// Minimal time struct — avoids chrono dependency.
struct SimpleTime {
    hour: u8,
    minute: u8,
    weekday: Weekday,
}

/// Convert a `u64` Unix-seconds value to `libc::time_t`, saturating instead of
/// wrapping on 32-bit targets. Protects against the 2038 wraparound when
/// `time_t` is `i32`; on 64-bit Linux this is a lossless conversion.
fn saturating_time_t(secs: u64) -> libc::time_t {
    libc::time_t::try_from(secs).unwrap_or(libc::time_t::MAX)
}

/// Get current local time without chrono.
fn chrono_free_now() -> SimpleTime {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Get local timezone offset via libc
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let t = saturating_time_t(secs);
        libc::localtime_r(&t, &mut tm);

        SimpleTime {
            hour: tm.tm_hour as u8,
            minute: tm.tm_min as u8,
            weekday: match tm.tm_wday {
                0 => Weekday::Sun,
                1 => Weekday::Mon,
                2 => Weekday::Tue,
                3 => Weekday::Wed,
                4 => Weekday::Thu,
                5 => Weekday::Fri,
                6 => Weekday::Sat,
                _ => Weekday::Mon,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::ipc::Target;

    fn entry(hour: u8, minute: u8, days: Vec<Weekday>, action: ScheduleAction) -> ScheduleEntry {
        ScheduleEntry {
            hour,
            minute,
            days,
            action,
        }
    }

    #[test]
    fn no_entries_no_commands() {
        let mut runner = ScheduleRunner::new();
        let commands = runner.evaluate(&[GroupId::new(1), GroupId::new(2), GroupId::new(3)]);
        assert!(commands.is_empty());
        assert!(!runner.is_active());
    }

    #[test]
    fn add_and_remove() {
        let mut runner = ScheduleRunner::new();
        runner.add(entry(8, 0, vec![], ScheduleAction::SwitchProfile("quiet".into())));
        assert!(runner.is_active());
        assert_eq!(runner.entries().len(), 1);

        assert!(runner.remove(0));
        assert!(!runner.is_active());
        assert!(!runner.remove(0)); // out of bounds
    }

    #[test]
    fn clear_removes_all() {
        let mut runner = ScheduleRunner::new();
        runner.add(entry(8, 0, vec![], ScheduleAction::SwitchProfile("a".into())));
        runner.add(entry(9, 0, vec![], ScheduleAction::SwitchProfile("b".into())));
        assert_eq!(runner.entries().len(), 2);

        runner.clear();
        assert!(!runner.is_active());
    }

    #[test]
    fn load_replaces() {
        let mut runner = ScheduleRunner::new();
        runner.add(entry(8, 0, vec![], ScheduleAction::SwitchProfile("old".into())));
        runner.load(vec![entry(
            10,
            30,
            vec![Weekday::Mon],
            ScheduleAction::SwitchProfile("new".into()),
        )]);
        assert_eq!(runner.entries().len(), 1);
        assert_eq!(runner.entries()[0].hour, 10);
    }

    #[test]
    fn resolve_target_all() {
        let all = vec![GroupId::new(1), GroupId::new(2), GroupId::new(3)];
        let groups = resolve_target(&Target::All, &all);
        assert_eq!(groups, all);
    }

    #[test]
    fn resolve_target_single() {
        let all = vec![GroupId::new(1), GroupId::new(2), GroupId::new(3)];
        let groups = resolve_target(&Target::Group(GroupId::new(5)), &all);
        assert_eq!(groups, vec![GroupId::new(5)]);
    }

    #[test]
    fn resolve_target_multiple() {
        let all = vec![GroupId::new(1), GroupId::new(2), GroupId::new(3)];
        let groups = resolve_target(&Target::Groups(vec![GroupId::new(2), GroupId::new(4)]), &all);
        assert_eq!(groups, vec![GroupId::new(2), GroupId::new(4)]);
    }

    #[test]
    fn saturating_time_t_handles_boundaries() {
        assert_eq!(saturating_time_t(0), 0 as libc::time_t);
        let mid = 1_000_000_000u64;
        assert_eq!(saturating_time_t(mid), mid as libc::time_t);
        assert_eq!(saturating_time_t(u64::MAX), libc::time_t::MAX);

        // 2038-01-19 + 1 second — the boundary this helper exists to handle.
        let just_past_i32 = (i32::MAX as u64) + 1;
        let result = saturating_time_t(just_past_i32);
        #[cfg(target_pointer_width = "64")]
        assert_eq!(result, just_past_i32 as libc::time_t);
        #[cfg(target_pointer_width = "32")]
        assert_eq!(result, libc::time_t::MAX);
    }
}
