//! Wires the LED editor tab callbacks.
//! Zone mode: per-fan inner/outer zone colors → RgbMode::Composed.
//! Per-LED mode: individual LED colors rendered as fan visuals → RgbMode::PerLed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use slint::ComponentHandle;

use crate::bridge::BridgeHandle;
use crate::fan_render::{self, FanRenderResult, LedHit, LedZone};
use crate::AppWindow;
use frgb_model::GroupId;

// ---------------------------------------------------------------------------
// Zone mode state
// ---------------------------------------------------------------------------

struct FanZone {
    inner: [u8; 3],
    outer: [u8; 3],
}

type ZoneMap = Arc<Mutex<HashMap<(GroupId, i32), FanZone>>>;

// ---------------------------------------------------------------------------
// Per-LED mode state
// ---------------------------------------------------------------------------

struct FanLedColors {
    inner: Vec<frgb_model::rgb::Rgb>,
    outer: Vec<frgb_model::rgb::Rgb>,
}

#[derive(Default)]
pub struct PerLedState {
    /// Per-fan LED colors. Key: (group_id, fan_index).
    colors: HashMap<(GroupId, i32), FanLedColors>,
    /// Cached render results for hit-testing. Key: (group_id, fan_index).
    renders: HashMap<(GroupId, i32), FanRenderResult>,
    /// Current selection.
    selected: Option<(GroupId, i32, LedHit)>, // (group_id, fan_index, hit)
}

pub type LedState = Arc<Mutex<PerLedState>>;

/// Seed LED state from daemon device status.
/// Extracts zone/LED colors from current RgbMode for each group.
pub fn sync_from_status(state: &LedState, groups: &[frgb_model::config::GroupStatus]) {
    use frgb_rgb::layout::LedLayout;

    let mut st = state.lock().unwrap();
    for gs in groups {
        let layout = LedLayout::for_device(gs.group.device_type);
        sync_mode_into(
            &mut st.colors,
            gs.group.id,
            gs.group.fan_count as i32,
            layout.inner_count as usize,
            layout.outer_count as usize,
            &gs.rgb,
        );
    }
}

/// Sync a single group from an RgbChanged event.
pub fn sync_from_event(
    state: &LedState,
    group_id: GroupId,
    mode: &frgb_model::rgb::RgbMode,
    device_type: frgb_model::device::DeviceType,
    fan_count: u8,
) {
    let layout = frgb_rgb::layout::LedLayout::for_device(device_type);

    let mut st = state.lock().unwrap();
    sync_mode_into(
        &mut st.colors,
        group_id,
        fan_count as i32,
        layout.inner_count as usize,
        layout.outer_count as usize,
        mode,
    );
}

fn sync_mode_into(
    colors: &mut HashMap<(GroupId, i32), FanLedColors>,
    gid: GroupId,
    fan_count: i32,
    inner_n: usize,
    outer_n: usize,
    mode: &frgb_model::rgb::RgbMode,
) {
    use frgb_model::rgb::{Rgb, RgbMode};

    match mode {
        RgbMode::Off => {
            for fi in 0..fan_count {
                colors.insert(
                    (gid, fi),
                    FanLedColors {
                        inner: vec![Rgb::BLACK; inner_n],
                        outer: vec![Rgb::BLACK; outer_n],
                    },
                );
            }
        }
        RgbMode::Static { color, .. } => {
            for fi in 0..fan_count {
                colors.insert(
                    (gid, fi),
                    FanLedColors {
                        inner: vec![*color; inner_n],
                        outer: vec![*color; outer_n],
                    },
                );
            }
        }
        RgbMode::Composed(specs) => {
            for fi in 0..fan_count {
                let spec = specs.get(fi as usize).or(specs.last());
                let (ic, oc) = match spec {
                    Some(s) => (zone_to_color(&s.inner), zone_to_color(&s.outer)),
                    None => (Rgb::BLACK, Rgb::BLACK),
                };
                colors.insert(
                    (gid, fi),
                    FanLedColors {
                        inner: vec![ic; inner_n],
                        outer: vec![oc; outer_n],
                    },
                );
            }
        }
        RgbMode::PerFan(assignments) => {
            for fi in 0..fan_count {
                let a = assignments.get(fi as usize).or(assignments.last());
                let (ic, oc) = match a {
                    Some(a) => (a.inner.unwrap_or(Rgb::BLACK), a.outer.unwrap_or(Rgb::BLACK)),
                    None => (Rgb::BLACK, Rgb::BLACK),
                };
                colors.insert(
                    (gid, fi),
                    FanLedColors {
                        inner: vec![ic; inner_n],
                        outer: vec![oc; outer_n],
                    },
                );
            }
        }
        RgbMode::PerLed(assignments) => {
            for fi in 0..fan_count {
                let a = assignments.get(fi as usize).or(assignments.last());
                let (inner, outer) = match a {
                    Some(a) => {
                        let mut iv = a.inner.clone();
                        let mut ov = a.outer.clone();
                        iv.resize(inner_n, Rgb::BLACK);
                        ov.resize(outer_n, Rgb::BLACK);
                        (iv, ov)
                    }
                    None => (vec![Rgb::BLACK; inner_n], vec![Rgb::BLACK; outer_n]),
                };
                colors.insert((gid, fi), FanLedColors { inner, outer });
            }
        }
        // Effect/TempRgb — can't extract per-LED colors meaningfully
        _ => {}
    }
}

fn zone_to_color(zone: &frgb_model::rgb::ZoneSource) -> frgb_model::rgb::Rgb {
    match zone {
        frgb_model::rgb::ZoneSource::Color { color, .. } => *color,
        _ => frgb_model::rgb::Rgb::BLACK,
    }
}

/// Re-render fan images for all fans in the group and push to UI.
pub fn render_and_push(window: &AppWindow, state: &LedState, group_id: GroupId) {
    use slint::Model;
    let model = window.get_devices();
    let group_data =
        (0..model.row_count()).find_map(|i| model.row_data(i).filter(|g| g.group_id == group_id.value() as i32));

    let (fan_count, device_type) = match group_data {
        Some(g) => (g.fan_count, crate::convert::device_type_from_display(&g.device_type)),
        None => return,
    };

    let mut st = state.lock().unwrap();
    let selected = st.selected;

    for fan_idx in 0..fan_count {
        let key = (group_id, fan_idx);
        let fc = st.colors.entry(key).or_insert_with(|| FanLedColors {
            inner: vec![frgb_model::rgb::Rgb::BLACK; 64],
            outer: vec![frgb_model::rgb::Rgb::BLACK; 64],
        });

        let sel_for_fan = selected
            .filter(|&(gid, fi, _)| gid == group_id && fi == fan_idx)
            .map(|(_, _, hit)| hit);

        let result = fan_render::render_fan(device_type, &fc.inner, &fc.outer, sel_for_fan);
        let image = fan_render::to_slint_image(&result);
        st.renders.insert(key, result);

        match fan_idx {
            0 => window.set_fan0_image(image),
            1 => window.set_fan1_image(image),
            2 => window.set_fan2_image(image),
            3 => window.set_fan3_image(image),
            _ => {}
        }
    }
}

fn show_status(w: &slint::Weak<AppWindow>, msg: impl Into<slint::SharedString>) {
    let msg = msg.into();
    let w = w.clone();
    slint::invoke_from_event_loop(move || {
        if let Some(window) = w.upgrade() {
            window.global::<crate::UiState>().set_daemon_status(msg);
        }
    })
    .ok();
}

pub fn fetch_led_presets(window: &AppWindow, bridge: &BridgeHandle) {
    let w = window.as_weak();
    bridge.call(frgb_ipc::Request::ListLedPresets, move |resp| {
        if let frgb_ipc::Response::LedPresets(presets) = resp {
            let names: Vec<slint::SharedString> = presets
                .iter()
                .map(|p| slint::SharedString::from(p.name.as_str()))
                .collect();
            slint::invoke_from_event_loop(move || {
                if let Some(window) = w.upgrade() {
                    window.set_led_preset_names(slint::ModelRc::new(slint::VecModel::from(names)));
                }
            })
            .ok();
        }
    });
}

pub fn wire_presets(window: &AppWindow, bridge: &BridgeHandle, led_state: &LedState) {
    // Save
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        let state = led_state.clone();
        window.on_save_led_preset(move |group_id, name| {
            let name_str = name.to_string();
            if name_str.is_empty() {
                return;
            }
            let gid = GroupId::new(group_id as u8);

            // Get device info for this group
            let (device_type, fan_count) = {
                if let Some(window) = w.upgrade() {
                    use slint::Model;
                    let model = window.get_devices();
                    (0..model.row_count())
                        .find_map(|i| model.row_data(i).filter(|g| g.group_id == group_id))
                        .map(|g| {
                            (
                                crate::convert::device_type_from_display(&g.device_type),
                                g.fan_count as u8,
                            )
                        })
                        .unwrap_or((frgb_model::device::DeviceType::ClWireless, 0))
                } else {
                    return;
                }
            };

            // Extract current colors as assignments
            let st = state.lock().unwrap();
            let assignments: Vec<frgb_model::rgb::FanLedAssignment> = (0..fan_count as i32)
                .map(|fi| {
                    st.colors
                        .get(&(gid, fi))
                        .map(|fc| frgb_model::rgb::FanLedAssignment {
                            inner: fc.inner.clone(),
                            outer: fc.outer.clone(),
                        })
                        .unwrap_or_else(|| frgb_model::rgb::FanLedAssignment {
                            inner: vec![],
                            outer: vec![],
                        })
                })
                .collect();
            drop(st);

            let validated_name = match frgb_model::ValidatedName::new(name_str) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("invalid preset name: {e}");
                    return;
                }
            };
            let preset = frgb_model::config::LedPreset {
                name: validated_name,
                group_device_type: device_type,
                fan_count,
                assignments,
            };

            let w2 = w.clone();
            let b2 = bridge.clone();
            bridge.call(frgb_ipc::Request::SaveLedPreset { preset }, move |resp| match resp {
                frgb_ipc::Response::Ok => {
                    show_status(&w2, "LED preset saved");
                    slint::invoke_from_event_loop({
                        let w3 = w2.clone();
                        let b3 = b2.clone();
                        move || {
                            if let Some(window) = w3.upgrade() {
                                fetch_led_presets(&window, &b3);
                            }
                        }
                    })
                    .ok();
                }
                frgb_ipc::Response::Error(e) => {
                    show_status(&w2, format!("Save failed: {e}"));
                }
                _ => {}
            });
        });
    }

    // Delete
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        window.on_delete_led_preset(move |name| {
            let name_str = name.to_string();
            if name_str.is_empty() {
                return;
            }
            let w2 = w.clone();
            let b2 = bridge.clone();
            bridge.call(
                frgb_ipc::Request::DeleteLedPreset { name: name_str },
                move |resp| match resp {
                    frgb_ipc::Response::Ok => {
                        show_status(&w2, "LED preset deleted");
                        slint::invoke_from_event_loop({
                            let w3 = w2.clone();
                            let b3 = b2.clone();
                            move || {
                                if let Some(window) = w3.upgrade() {
                                    fetch_led_presets(&window, &b3);
                                }
                            }
                        })
                        .ok();
                    }
                    frgb_ipc::Response::Error(e) => {
                        show_status(&w2, format!("Delete failed: {e}"));
                    }
                    _ => {}
                },
            );
        });
    }

    // Load
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        let state = led_state.clone();
        window.on_load_led_preset(move |group_id, name| {
            let name_str = name.to_string();
            let gid = GroupId::new(group_id as u8);
            let w2 = w.clone();
            let state2 = state.clone();
            bridge.call(frgb_ipc::Request::ListLedPresets, move |resp| {
                if let frgb_ipc::Response::LedPresets(presets) = resp {
                    if let Some(preset) = presets.iter().find(|p| p.name == name_str) {
                        // Apply preset colors to state
                        let mut st = state2.lock().unwrap();
                        for (fi, assignment) in preset.assignments.iter().enumerate() {
                            st.colors.insert(
                                (gid, fi as i32),
                                FanLedColors {
                                    inner: assignment.inner.clone(),
                                    outer: assignment.outer.clone(),
                                },
                            );
                        }
                        drop(st);

                        let state3 = state2.clone();
                        let name_copy = name_str.clone();
                        slint::invoke_from_event_loop(move || {
                            if let Some(window) = w2.upgrade() {
                                show_status(&w2, format!("Loaded preset '{name_copy}'"));
                                render_and_push(&window, &state3, gid);
                            }
                        })
                        .ok();
                    }
                }
            });
        });
    }
}

pub fn wire(window: &AppWindow, bridge: &BridgeHandle, led_state: &LedState) {
    let zones: ZoneMap = Arc::new(Mutex::new(HashMap::new()));
    let led_state = led_state.clone();

    // --- Zone mode callbacks (unchanged) ---

    {
        let zones = zones.clone();
        window.on_fan_zone_changed(move |group_id, fan_index, ir, ig, ib, or_, og, ob| {
            let key = (GroupId::new(group_id as u8), fan_index);
            zones.lock().unwrap().insert(
                key,
                FanZone {
                    inner: [ir.clamp(0, 255) as u8, ig.clamp(0, 255) as u8, ib.clamp(0, 255) as u8],
                    outer: [or_.clamp(0, 255) as u8, og.clamp(0, 255) as u8, ob.clamp(0, 255) as u8],
                },
            );
        });
    }

    {
        let bridge = bridge.clone();
        let zones = zones.clone();
        let w = window.as_weak();
        window.on_apply_led(move |group_id| {
            if let Some(window) = w.upgrade() {
                use slint::Model;
                let gid = GroupId::new(group_id as u8);
                let model = window.get_devices();
                let fan_count = (0..model.row_count())
                    .find_map(|i| model.row_data(i).filter(|g| g.group_id == group_id))
                    .map_or(0, |g| g.fan_count) as usize;

                let map = zones.lock().unwrap();
                let specs: Vec<frgb_model::rgb::FanZoneSpec> = (0..fan_count as i32)
                    .map(|fi| {
                        let zone = map.get(&(gid, fi));
                        let (inner, outer) = match zone {
                            Some(z) => (z.inner, z.outer),
                            None => ([254, 0, 128], [254, 0, 128]),
                        };
                        frgb_model::rgb::FanZoneSpec {
                            inner: frgb_model::rgb::ZoneSource::Color {
                                color: frgb_model::rgb::Rgb {
                                    r: inner[0],
                                    g: inner[1],
                                    b: inner[2],
                                },
                                brightness: frgb_model::Brightness::new(255),
                            },
                            outer: frgb_model::rgb::ZoneSource::Color {
                                color: frgb_model::rgb::Rgb {
                                    r: outer[0],
                                    g: outer[1],
                                    b: outer[2],
                                },
                                brightness: frgb_model::Brightness::new(255),
                            },
                        }
                    })
                    .collect();
                drop(map);

                bridge.send(frgb_ipc::Request::SetRgb {
                    group: gid,
                    mode: frgb_model::rgb::RgbMode::Composed(specs),
                });
            }
        });
    }

    // --- Per-LED mode callbacks ---

    // Fan image clicked → hit-test → update selection → re-render
    {
        let state = led_state.clone();
        let w = window.as_weak();
        window.on_fan_image_clicked(move |group_id, fan_index, x, y| {
            let gid = GroupId::new(group_id as u8);
            let fi = fan_index;

            // Hit-test against cached render
            let hit = {
                let st = state.lock().unwrap();
                st.renders.get(&(gid, fi)).and_then(|r| fan_render::hit_test(r, x, y))
            };

            if let Some(hit) = hit {
                // Update selection
                {
                    let mut st = state.lock().unwrap();
                    st.selected = Some((gid, fi, hit));
                }

                if let Some(window) = w.upgrade() {
                    // Read current color of selected LED
                    let color = {
                        let st = state.lock().unwrap();
                        st.colors
                            .get(&(gid, fi))
                            .map(|fc| {
                                let vec = match hit.zone {
                                    LedZone::Inner => &fc.inner,
                                    LedZone::Outer => &fc.outer,
                                };
                                vec.get(hit.index).copied().unwrap_or(frgb_model::rgb::Rgb::BLACK)
                            })
                            .unwrap_or(frgb_model::rgb::Rgb::BLACK)
                    };

                    // Push selection state to UI
                    let zone_str = match hit.zone {
                        LedZone::Inner => "inner",
                        LedZone::Outer => "outer",
                    };
                    window.set_led_sel_zone(slint::SharedString::from(zone_str));
                    window.set_led_sel_fan(fi);
                    window.set_led_sel_led(hit.index as i32);
                    window.set_led_edit_r(color.r as i32);
                    window.set_led_edit_g(color.g as i32);
                    window.set_led_edit_b(color.b as i32);

                    // Re-render with selection highlight
                    render_and_push(&window, &state, gid);
                }
            }
        });
    }

    // LED color changed → update state → re-render
    {
        let state = led_state.clone();
        let w = window.as_weak();
        window.on_led_color_changed(move |group_id, fan_index, zone, led_index, r, g, b| {
            let gid = GroupId::new(group_id as u8);
            let color = frgb_model::rgb::Rgb {
                r: r.clamp(0, 255) as u8,
                g: g.clamp(0, 255) as u8,
                b: b.clamp(0, 255) as u8,
            };
            let zone_str = zone.to_string();

            {
                let mut st = state.lock().unwrap();
                let fc = st.colors.entry((gid, fan_index)).or_insert_with(|| FanLedColors {
                    inner: vec![frgb_model::rgb::Rgb::BLACK; 64],
                    outer: vec![frgb_model::rgb::Rgb::BLACK; 64],
                });
                let vec = if zone_str == "inner" {
                    &mut fc.inner
                } else {
                    &mut fc.outer
                };
                let idx = led_index as usize;
                if idx >= vec.len() {
                    vec.resize(idx + 1, frgb_model::rgb::Rgb::BLACK);
                }
                vec[idx] = color;
            }

            if let Some(window) = w.upgrade() {
                render_and_push(&window, &state, gid);
            }
        });
    }

    // Apply Per-LED → build PerLed RgbMode and send
    {
        let bridge = bridge.clone();
        let state = led_state.clone();
        let w = window.as_weak();
        window.on_apply_per_led(move |group_id| {
            if let Some(window) = w.upgrade() {
                use slint::Model;
                let gid = GroupId::new(group_id as u8);
                let model = window.get_devices();
                let group_data =
                    (0..model.row_count()).find_map(|i| model.row_data(i).filter(|g| g.group_id == group_id));

                let (fan_count, inner_leds, outer_leds) = match group_data {
                    Some(g) => (g.fan_count as usize, g.inner_leds as usize, g.outer_leds as usize),
                    None => return,
                };

                let st = state.lock().unwrap();
                let assignments: Vec<frgb_model::rgb::FanLedAssignment> = (0..fan_count as i32)
                    .map(|fi| {
                        let fc = st.colors.get(&(gid, fi));
                        let inner: Vec<frgb_model::rgb::Rgb> = (0..inner_leds)
                            .map(|i| {
                                fc.and_then(|c| c.inner.get(i).copied())
                                    .unwrap_or(frgb_model::rgb::Rgb::BLACK)
                            })
                            .collect();
                        let outer: Vec<frgb_model::rgb::Rgb> = (0..outer_leds)
                            .map(|i| {
                                fc.and_then(|c| c.outer.get(i).copied())
                                    .unwrap_or(frgb_model::rgb::Rgb::BLACK)
                            })
                            .collect();
                        frgb_model::rgb::FanLedAssignment { inner, outer }
                    })
                    .collect();
                drop(st);

                bridge.send(frgb_ipc::Request::SetRgb {
                    group: gid,
                    mode: frgb_model::rgb::RgbMode::PerLed(assignments),
                });
            }
        });
    }

    // Per-LED mode activated → re-render fan visuals from current state
    {
        let state = led_state.clone();
        let w = window.as_weak();
        window.on_per_led_activated(move |group_id| {
            if let Some(window) = w.upgrade() {
                render_and_push(&window, &state, GroupId::new(group_id as u8));
            }
        });
    }
}
