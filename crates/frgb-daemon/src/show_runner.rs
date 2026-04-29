//! Show runner — daemon-driven effect cycling and custom sequences.
//!
//! Per-group playback state machine. Each tick advances timers; step
//! transitions produce StepActions that the engine applies via System.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use frgb_model::ipc::Event;
use frgb_model::rgb::{RgbMode, Ring};
use frgb_model::show::{EffectCycle, EffectCycleStep, Playback, Scene, Sequence};
use frgb_model::GroupId;

const EFFECT_CYCLE_NAME: &str = "effect_cycle";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Manages active sequence/cycle playback per group.
pub struct ShowRunner {
    active: HashMap<GroupId, ActivePlayback>,
    /// Named sequences cached from config (avoids per-request disk I/O).
    sequences: HashMap<String, Arc<Sequence>>,
}

/// Action the engine should take after a tick.
pub struct StepAction {
    pub group: GroupId,
    pub scene: Scene,
}

impl ShowRunner {
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
            sequences: HashMap::new(),
        }
    }

    /// Cache sequences from config (call at startup and on config reload).
    pub fn load_sequences(&mut self, config: &frgb_model::config::Config) {
        self.sequences.clear();
        for seq in &config.sequences {
            self.sequences.insert(seq.name.to_string(), Arc::new(seq.clone()));
        }
        for profile in &config.profiles {
            for seq in &profile.sequences {
                self.sequences
                    .entry(seq.name.to_string())
                    .or_insert_with(|| Arc::new(seq.clone()));
            }
        }
    }

    /// Look up a cached sequence by name.
    pub fn get_sequence(&self, name: &str) -> Option<&Arc<Sequence>> {
        self.sequences.get(name)
    }

    /// Start an effect cycle on the given groups (shared via Arc).
    pub fn start_cycle(&mut self, groups: &[GroupId], cycle: EffectCycle) {
        if cycle.steps.is_empty() {
            return;
        }
        let shared = Arc::new(cycle);
        for &group in groups {
            self.active
                .insert(group, ActivePlayback::new_cycle(Arc::clone(&shared)));
        }
    }

    /// Start a sequence on the given groups (shared via Arc).
    pub fn start_sequence(&mut self, groups: &[GroupId], sequence: Arc<Sequence>) {
        if sequence.steps.is_empty() {
            return;
        }
        for &group in groups {
            self.active
                .insert(group, ActivePlayback::new_sequence(Arc::clone(&sequence)));
        }
    }

    /// Stop playback for the given groups. If empty, stops all.
    pub fn stop(&mut self, groups: &[GroupId]) -> Vec<Event> {
        let mut events = Vec::new();
        if groups.is_empty() {
            for (_, pb) in self.active.drain() {
                events.push(Event::SequenceEnded { name: pb.name() });
            }
        } else {
            for &group in groups {
                if let Some(pb) = self.active.remove(&group) {
                    events.push(Event::SequenceEnded { name: pb.name() });
                }
            }
        }
        events
    }

    /// Advance all active playbacks by `elapsed`. Returns actions for step transitions.
    pub fn tick(&mut self, elapsed: Duration) -> (Vec<StepAction>, Vec<Event>) {
        if self.active.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let mut actions = Vec::new();
        let mut events = Vec::new();
        let mut finished = Vec::new();

        for (&group, playback) in self.active.iter_mut() {
            match playback.advance(elapsed) {
                AdvanceResult::Hold => {}
                AdvanceResult::NewStep { scene, step_index } => {
                    actions.push(StepAction { group, scene });
                    events.push(Event::SequenceStep { group, step_index });
                }
                AdvanceResult::Ended { name } => {
                    finished.push(group);
                    events.push(Event::SequenceEnded { name });
                }
            }
        }

        for group in finished {
            self.active.remove(&group);
        }

        (actions, events)
    }

    pub fn is_active(&self) -> bool {
        !self.active.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Playback state machine — shared between Cycle and Sequence
// ---------------------------------------------------------------------------

/// Step progression state, independent of what kind of content is playing.
struct PlaybackState {
    step_index: usize,
    step_elapsed: Duration,
    /// +1 forward, -1 backward (for PingPong). Stored as i8 for arithmetic.
    direction: i8,
    loops_remaining: Option<u16>,
    first_step: bool,
}

impl PlaybackState {
    fn new(playback: &Playback) -> Self {
        Self {
            step_index: 0,
            step_elapsed: Duration::ZERO,
            direction: 1,
            loops_remaining: match playback {
                Playback::Count(n) => Some(*n),
                _ => None,
            },
            first_step: true,
        }
    }

    /// Advance the state machine. Returns the new step index on transition, or None on end.
    fn advance(
        &mut self,
        elapsed: Duration,
        step_count: usize,
        step_duration: Duration,
        playback: &Playback,
    ) -> StepResult {
        if self.first_step {
            self.first_step = false;
            return StepResult::NewStep(self.step_index);
        }

        self.step_elapsed += elapsed;
        if self.step_elapsed < step_duration {
            return StepResult::Hold;
        }

        self.step_elapsed -= step_duration;
        match self.advance_index(step_count, playback) {
            Some(new_idx) => {
                self.step_index = new_idx;
                StepResult::NewStep(new_idx)
            }
            None => StepResult::Ended,
        }
    }

    fn advance_index(&mut self, len: usize, playback: &Playback) -> Option<usize> {
        if len <= 1 {
            return match playback {
                Playback::Once => None,
                Playback::Loop | Playback::PingPong => Some(0),
                Playback::Count(_) => {
                    if let Some(ref mut n) = self.loops_remaining {
                        if *n <= 1 {
                            return None;
                        }
                        *n -= 1;
                    }
                    Some(0)
                }
            };
        }

        let next = self.step_index as i64 + self.direction as i64;
        if next >= 0 && (next as usize) < len {
            return Some(next as usize);
        }

        // Hit a boundary
        match playback {
            Playback::Once => None,
            Playback::Loop => Some(if self.direction > 0 { 0 } else { len - 1 }),
            Playback::PingPong => {
                self.direction = -self.direction;
                let bounced = self.step_index as i64 + self.direction as i64;
                Some(if bounced >= 0 && (bounced as usize) < len {
                    bounced as usize
                } else {
                    self.step_index
                })
            }
            Playback::Count(_) => {
                if let Some(ref mut n) = self.loops_remaining {
                    if *n <= 1 {
                        return None;
                    }
                    *n -= 1;
                }
                Some(if self.direction > 0 { 0 } else { len - 1 })
            }
        }
    }
}

enum StepResult {
    Hold,
    NewStep(usize),
    Ended,
}

// ---------------------------------------------------------------------------
// ActivePlayback — thin wrapper over PlaybackState + content source
// ---------------------------------------------------------------------------

enum ActivePlayback {
    Cycle {
        cycle: Arc<EffectCycle>,
        state: PlaybackState,
    },
    Sequence {
        sequence: Arc<Sequence>,
        state: PlaybackState,
    },
}

enum AdvanceResult {
    Hold,
    NewStep { scene: Scene, step_index: usize },
    Ended { name: String },
}

impl ActivePlayback {
    fn new_cycle(cycle: Arc<EffectCycle>) -> Self {
        let state = PlaybackState::new(&cycle.playback);
        Self::Cycle { cycle, state }
    }

    fn new_sequence(sequence: Arc<Sequence>) -> Self {
        let state = PlaybackState::new(&sequence.playback);
        Self::Sequence { sequence, state }
    }

    fn name(&self) -> String {
        match self {
            Self::Cycle { .. } => EFFECT_CYCLE_NAME.into(),
            Self::Sequence { sequence, .. } => sequence.name.to_string(),
        }
    }

    fn advance(&mut self, elapsed: Duration) -> AdvanceResult {
        match self {
            Self::Cycle { cycle, state } => {
                let duration = Duration::from_millis(cycle.steps[state.step_index].duration_ms as u64);
                match state.advance(elapsed, cycle.steps.len(), duration, &cycle.playback) {
                    StepResult::Hold => AdvanceResult::Hold,
                    StepResult::NewStep(idx) => AdvanceResult::NewStep {
                        scene: cycle_step_to_scene(&cycle.steps[idx]),
                        step_index: idx,
                    },
                    StepResult::Ended => AdvanceResult::Ended {
                        name: EFFECT_CYCLE_NAME.into(),
                    },
                }
            }
            Self::Sequence { sequence, state } => {
                let duration = Duration::from_millis(sequence.steps[state.step_index].duration_ms as u64);
                match state.advance(elapsed, sequence.steps.len(), duration, &sequence.playback) {
                    StepResult::Hold => AdvanceResult::Hold,
                    StepResult::NewStep(idx) => AdvanceResult::NewStep {
                        scene: sequence.steps[idx].scene.clone(),
                        step_index: idx,
                    },
                    StepResult::Ended => AdvanceResult::Ended {
                        name: sequence.name.to_string(),
                    },
                }
            }
        }
    }
}

fn cycle_step_to_scene(step: &EffectCycleStep) -> Scene {
    Scene {
        rgb: RgbMode::Effect {
            effect: step.effect,
            params: step.params.clone(),
            ring: Ring::Both,
        },
        speed: None,
        lcd: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::effect::Effect;
    use frgb_model::rgb::EffectParams;

    fn test_cycle(steps: usize, duration_ms: u32, playback: Playback) -> EffectCycle {
        EffectCycle {
            steps: (0..steps)
                .map(|_| EffectCycleStep {
                    effect: Effect::Rainbow,
                    params: EffectParams::default(),
                    duration_ms,
                    merge: false,
                })
                .collect(),
            playback,
        }
    }

    #[test]
    fn cycle_emits_first_step_immediately() {
        let mut runner = ShowRunner::new();
        runner.start_cycle(&[GroupId::new(1)], test_cycle(3, 5000, Playback::Loop));

        let (actions, events) = runner.tick(Duration::from_millis(0));
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].group, GroupId::new(1));
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn cycle_holds_during_step() {
        let mut runner = ShowRunner::new();
        runner.start_cycle(&[GroupId::new(1)], test_cycle(3, 5000, Playback::Loop));

        runner.tick(Duration::ZERO);
        let (actions, _) = runner.tick(Duration::from_millis(2000));
        assert!(actions.is_empty());
    }

    #[test]
    fn cycle_advances_after_duration() {
        let mut runner = ShowRunner::new();
        runner.start_cycle(&[GroupId::new(1)], test_cycle(3, 100, Playback::Loop));

        runner.tick(Duration::ZERO);
        let (actions, events) = runner.tick(Duration::from_millis(100));
        assert_eq!(actions.len(), 1);
        if let Event::SequenceStep { step_index, .. } = &events[0] {
            assert_eq!(*step_index, 1);
        }
    }

    #[test]
    fn cycle_loops() {
        let mut runner = ShowRunner::new();
        runner.start_cycle(&[GroupId::new(1)], test_cycle(2, 50, Playback::Loop));

        runner.tick(Duration::ZERO);
        runner.tick(Duration::from_millis(50));
        let (_, events) = runner.tick(Duration::from_millis(50));

        if let Event::SequenceStep { step_index, .. } = &events[0] {
            assert_eq!(*step_index, 0);
        }
    }

    #[test]
    fn cycle_once_ends() {
        let mut runner = ShowRunner::new();
        runner.start_cycle(&[GroupId::new(1)], test_cycle(2, 50, Playback::Once));

        runner.tick(Duration::ZERO);
        runner.tick(Duration::from_millis(50));
        let (_, events) = runner.tick(Duration::from_millis(50));
        assert!(events.iter().any(|e| matches!(e, Event::SequenceEnded { .. })));
        assert!(!runner.is_active());
    }

    #[test]
    fn cycle_count_limited() {
        let mut runner = ShowRunner::new();
        runner.start_cycle(&[GroupId::new(1)], test_cycle(1, 50, Playback::Count(3)));

        runner.tick(Duration::ZERO);
        runner.tick(Duration::from_millis(50));
        runner.tick(Duration::from_millis(50));
        let (_, events) = runner.tick(Duration::from_millis(50));
        assert!(events.iter().any(|e| matches!(e, Event::SequenceEnded { .. })));
    }

    #[test]
    fn stop_emits_event() {
        let mut runner = ShowRunner::new();
        runner.start_cycle(&[GroupId::new(1), GroupId::new(2)], test_cycle(2, 5000, Playback::Loop));

        let events = runner.stop(&[GroupId::new(1)]);
        assert_eq!(events.len(), 1);
        assert!(runner.is_active());

        let events = runner.stop(&[]);
        assert_eq!(events.len(), 1);
        assert!(!runner.is_active());
    }

    #[test]
    fn empty_cycle_ignored() {
        let mut runner = ShowRunner::new();
        runner.start_cycle(
            &[GroupId::new(1)],
            EffectCycle {
                steps: vec![],
                playback: Playback::Loop,
            },
        );
        assert!(!runner.is_active());
    }

    #[test]
    fn multi_group_independent() {
        let mut runner = ShowRunner::new();
        runner.start_cycle(&[GroupId::new(1)], test_cycle(2, 100, Playback::Loop));
        runner.start_cycle(&[GroupId::new(2)], test_cycle(3, 200, Playback::Loop));

        runner.tick(Duration::ZERO);
        let (actions, _) = runner.tick(Duration::from_millis(100));
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].group, GroupId::new(1));
    }

    #[test]
    fn multi_group_shares_arc() {
        let mut runner = ShowRunner::new();
        let cycle = test_cycle(3, 100, Playback::Loop);
        runner.start_cycle(&[GroupId::new(1), GroupId::new(2), GroupId::new(3)], cycle);
        // All 3 groups active, sharing the same Arc<EffectCycle>
        assert!(runner.is_active());
        runner.tick(Duration::ZERO);
        let (actions, _) = runner.tick(Duration::from_millis(100));
        assert_eq!(actions.len(), 3); // all advance together
    }
}
