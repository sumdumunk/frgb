//! Wires the sequence editor page callbacks.
//! Fetches sequences on connect, supports create/edit/delete/play/stop.

use slint::ComponentHandle;

use crate::bridge::BridgeHandle;
use crate::AppWindow;

/// Fetch saved sequences from daemon and populate the UI list.
/// Intermediate Send-safe sequence data for cross-thread transfer.
struct SeqTransfer {
    name: String,
    step_count: i32,
    playback: String,
    steps: Vec<crate::SequenceStepData>,
}

pub fn fetch_sequences(window: &AppWindow, bridge: &BridgeHandle) {
    let w = window.as_weak();
    bridge.call(frgb_ipc::Request::ListSequences, move |resp| {
        if let frgb_ipc::Response::SequenceList(sequences) = resp {
            let transfers: Vec<SeqTransfer> = sequences
                .iter()
                .map(|s| SeqTransfer {
                    name: s.name.to_string(),
                    step_count: s.steps.len() as i32,
                    playback: playback_str(&s.playback).to_string(),
                    steps: s.steps.iter().map(step_to_slint).collect(),
                })
                .collect();
            slint::invoke_from_event_loop(move || {
                if let Some(window) = w.upgrade() {
                    let slint_seqs: Vec<crate::SequenceData> = transfers
                        .into_iter()
                        .map(|t| crate::SequenceData {
                            name: slint::SharedString::from(&t.name),
                            step_count: t.step_count,
                            playback: slint::SharedString::from(&t.playback),
                            steps: slint::ModelRc::new(slint::VecModel::from(t.steps)),
                        })
                        .collect();
                    window.set_sequences(slint::ModelRc::new(slint::VecModel::from(slint_seqs)));
                }
            })
            .ok();
        }
    });
}

pub fn wire(window: &AppWindow, bridge: &BridgeHandle) {
    // Save sequence
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        window.on_save_sequence(move |name, playback| {
            let name = name.to_string();
            if name.is_empty() {
                return;
            }
            let w2 = w.clone();
            let b2 = bridge.clone();

            // Read steps from UI
            let steps = if let Some(window) = w.upgrade() {
                use slint::Model;
                let model = window.get_seq_edit_steps();
                (0..model.row_count())
                    .filter_map(|i| model.row_data(i).map(|s| slint_to_step(&s)))
                    .collect()
            } else {
                vec![]
            };

            let validated_name = match frgb_model::ValidatedName::new(name.clone()) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("invalid sequence name: {e}");
                    return;
                }
            };
            let sequence = frgb_model::show::Sequence {
                name: validated_name,
                steps,
                playback: playback_from_str(&playback),
            };

            bridge.call(frgb_ipc::Request::SaveSequence { sequence }, move |resp| {
                if matches!(resp, frgb_ipc::Response::Ok) {
                    slint::invoke_from_event_loop(move || {
                        if let Some(window) = w2.upgrade() {
                            fetch_sequences(&window, &b2);
                        }
                    })
                    .ok();
                } else {
                    tracing::warn!("save sequence failed: {:?}", resp);
                }
            });
        });
    }

    // Delete sequence
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        window.on_delete_sequence(move |name| {
            let name = name.to_string();
            let w2 = w.clone();
            let b2 = bridge.clone();
            bridge.call(frgb_ipc::Request::DeleteSequence { name }, move |resp| {
                if matches!(resp, frgb_ipc::Response::Ok) {
                    slint::invoke_from_event_loop(move || {
                        if let Some(window) = w2.upgrade() {
                            window.set_selected_sequence(-1);
                            fetch_sequences(&window, &b2);
                        }
                    })
                    .ok();
                }
            });
        });
    }

    // Play sequence — only set active_show after daemon confirms start
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        window.on_play_sequence(move |name| {
            let w2 = w.clone();
            let show_name = name.clone();
            bridge.call(
                frgb_ipc::Request::StartSequence {
                    name: name.to_string(),
                    target: None,
                },
                move |resp| {
                    if matches!(resp, frgb_ipc::Response::Ok) {
                        slint::invoke_from_event_loop(move || {
                            if let Some(window) = w2.upgrade() {
                                window.set_active_show(show_name);
                            }
                        })
                        .ok();
                    } else {
                        tracing::warn!("start sequence failed: {:?}", resp);
                    }
                },
            );
        });
    }

    // Stop all
    {
        let bridge = bridge.clone();
        window.on_stop_all_sequences(move || {
            bridge.send(frgb_ipc::Request::StopAllSequences);
        });
    }

    // Add step
    {
        let w = window.as_weak();
        window.on_seq_add_step(move || {
            if let Some(window) = w.upgrade() {
                use slint::Model;
                let model = window.get_seq_edit_steps();
                let mut steps: Vec<crate::SequenceStepData> =
                    (0..model.row_count()).filter_map(|i| model.row_data(i)).collect();
                steps.push(crate::SequenceStepData {
                    effect_name: slint::SharedString::from("Rainbow"),
                    duration_ms: 5000,
                    transition: slint::SharedString::from("Cut"),
                    crossfade_ms: 500,
                });
                window.set_seq_edit_steps(slint::ModelRc::new(slint::VecModel::from(steps)));
            }
        });
    }

    // Remove step
    {
        let w = window.as_weak();
        window.on_seq_remove_step(move |index| {
            if let Some(window) = w.upgrade() {
                use slint::Model;
                let model = window.get_seq_edit_steps();
                let mut steps: Vec<crate::SequenceStepData> =
                    (0..model.row_count()).filter_map(|i| model.row_data(i)).collect();
                if (index as usize) < steps.len() {
                    steps.remove(index as usize);
                }
                window.set_seq_edit_steps(slint::ModelRc::new(slint::VecModel::from(steps)));
            }
        });
    }

    // Update step
    {
        let w = window.as_weak();
        window.on_seq_update_step(move |index, effect, duration, transition, crossfade| {
            if let Some(window) = w.upgrade() {
                use slint::Model;
                let model = window.get_seq_edit_steps();
                if let Some(mut step) = model.row_data(index as usize) {
                    step.effect_name = effect;
                    step.duration_ms = duration;
                    step.transition = transition;
                    step.crossfade_ms = crossfade;
                    model.set_row_data(index as usize, step);
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn playback_str(p: &frgb_model::show::Playback) -> &'static str {
    match p {
        frgb_model::show::Playback::Once => "Once",
        frgb_model::show::Playback::Loop => "Loop",
        frgb_model::show::Playback::PingPong => "PingPong",
        frgb_model::show::Playback::Count(_) => "Loop",
    }
}

fn playback_from_str(s: &str) -> frgb_model::show::Playback {
    match s {
        "Once" => frgb_model::show::Playback::Once,
        "PingPong" => frgb_model::show::Playback::PingPong,
        _ => frgb_model::show::Playback::Loop,
    }
}

fn step_to_slint(step: &frgb_model::show::SequenceStep) -> crate::SequenceStepData {
    let effect_name = match &step.scene.rgb {
        frgb_model::rgb::RgbMode::Effect { effect, .. } => crate::rgb_convert::effect_display_name(effect).to_string(),
        frgb_model::rgb::RgbMode::Static { .. } => "Static Color".to_string(),
        frgb_model::rgb::RgbMode::Off => "Off".to_string(),
        _ => "Custom".to_string(),
    };
    let (transition, crossfade_ms) = match step.transition {
        frgb_model::show::Transition::Cut => ("Cut".to_string(), 500),
        frgb_model::show::Transition::Crossfade { duration_ms } => ("Crossfade".to_string(), duration_ms as i32),
    };
    crate::SequenceStepData {
        effect_name: slint::SharedString::from(effect_name),
        duration_ms: step.duration_ms as i32,
        transition: slint::SharedString::from(transition),
        crossfade_ms,
    }
}

fn slint_to_step(step: &crate::SequenceStepData) -> frgb_model::show::SequenceStep {
    let effect = crate::rgb_convert::effect_from_display_name(&step.effect_name);
    let rgb = match effect {
        Some(e) => frgb_model::rgb::RgbMode::Effect {
            effect: e,
            params: frgb_model::rgb::EffectParams::default(),
            ring: frgb_model::rgb::Ring::Both,
        },
        None if step.effect_name == "Off" => frgb_model::rgb::RgbMode::Off,
        None => frgb_model::rgb::RgbMode::Off,
    };
    let transition = if step.transition == "Crossfade" {
        frgb_model::show::Transition::Crossfade {
            duration_ms: step.crossfade_ms.max(100) as u32,
        }
    } else {
        frgb_model::show::Transition::Cut
    };
    frgb_model::show::SequenceStep {
        target: None,
        scene: frgb_model::show::Scene {
            rgb,
            speed: None,
            lcd: None,
        },
        duration_ms: step.duration_ms.max(500) as u32,
        transition,
    }
}
