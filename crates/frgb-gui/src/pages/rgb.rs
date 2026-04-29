//! Wires the RGB page callbacks.
//!
//! Per-ring state: inner and outer rings are tracked independently per group.
//! When the user selects Inner/Outer, they edit that ring alone.  When Both
//! is selected, edits go to both rings simultaneously.  Apply always sends
//! a Composed mode with explicit inner + outer zone sources.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use slint::ComponentHandle;

use crate::bridge::BridgeHandle;
use crate::rgb_convert;
use crate::{AppWindow, UiState};
use frgb_model::GroupId;

/// Selected group index from UiState. `-1` means no selection.
///
/// Earlier builds read `window.get_selected_index()`, but that property is
/// never updated by the sidebar — the true selection lives in `UiState::
/// selected-group`. Reading the wrong source made the RGB tab always load
/// group 0's state regardless of what the user actually picked.
fn selected_group_index(window: &AppWindow) -> i32 {
    window.global::<UiState>().get_selected_group()
}

// ---------------------------------------------------------------------------
// Per-ring settings
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct RingSettings {
    mode: String,
    r: i32,
    g: i32,
    b: i32,
    effect: String,
    speed: i32,
    direction: i32,
    brightness: i32,
}

impl Default for RingSettings {
    fn default() -> Self {
        Self {
            mode: "off".into(),
            r: 255,
            g: 0,
            b: 128,
            effect: String::new(),
            speed: 3,
            direction: 0,
            brightness: 200,
        }
    }
}

// ---------------------------------------------------------------------------
// Per-group saved state (inner + outer independently)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct SavedRgbState {
    ring: i32, // 0=Inner, 1=Outer, 2=Both
    inner: RingSettings,
    outer: RingSettings,
}

impl Default for SavedRgbState {
    fn default() -> Self {
        Self {
            ring: 2,
            inner: RingSettings::default(),
            outer: RingSettings::default(),
        }
    }
}

/// Shared RGB state map — exposed so initial device status can seed it.
/// Internal state is opaque; use `sync_from_status` and `sync_from_event` to populate.
#[derive(Clone)]
pub struct RgbStateMap(Arc<Mutex<HashMap<GroupId, SavedRgbState>>>);

impl RgbStateMap {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::new())))
    }
}

type StateMap = RgbStateMap;

// ---------------------------------------------------------------------------
// ZoneSource → RingSettings conversion (inverse of zone_source())
// ---------------------------------------------------------------------------

/// Convert a ZoneSource into RingSettings.
/// Source: INFERRED — inverse of zone_source(), maps model types back to UI state strings.
fn ring_settings_from_zone(zone: &frgb_model::rgb::ZoneSource) -> RingSettings {
    use frgb_model::rgb::ZoneSource;
    match zone {
        ZoneSource::Color { color, brightness } => RingSettings {
            mode: "static".into(),
            r: color.r as i32,
            g: color.g as i32,
            b: color.b as i32,
            brightness: brightness.value() as i32,
            ..Default::default()
        },
        ZoneSource::Effect { effect, params } => RingSettings {
            mode: "effect".into(),
            r: params.color.map_or(255, |c| c.r as i32),
            g: params.color.map_or(0, |c| c.g as i32),
            b: params.color.map_or(128, |c| c.b as i32),
            effect: rgb_convert::effect_display_name(effect).to_string(),
            speed: params.speed as i32,
            direction: rgb_convert::direction_to_index(&params.direction),
            brightness: params.brightness.value() as i32,
        },
        ZoneSource::Off => RingSettings::default(),
    }
}

/// Convert an RgbMode from the daemon into a SavedRgbState.
/// Source: INFERRED — maps daemon RgbMode variants to the per-ring UI state model.
fn state_from_rgb_mode(mode: &frgb_model::rgb::RgbMode) -> SavedRgbState {
    use frgb_model::rgb::RgbMode;
    match mode {
        RgbMode::Off => SavedRgbState::default(),
        RgbMode::Static {
            ring,
            color,
            brightness,
        } => {
            let settings = RingSettings {
                mode: "static".into(),
                r: color.r as i32,
                g: color.g as i32,
                b: color.b as i32,
                brightness: brightness.value() as i32,
                ..Default::default()
            };
            match ring {
                frgb_model::rgb::Ring::Inner => SavedRgbState {
                    ring: 0,
                    inner: settings,
                    outer: RingSettings::default(),
                },
                frgb_model::rgb::Ring::Outer => SavedRgbState {
                    ring: 1,
                    inner: RingSettings::default(),
                    outer: settings,
                },
                frgb_model::rgb::Ring::Both => SavedRgbState {
                    ring: 2,
                    inner: settings.clone(),
                    outer: settings,
                },
            }
        }
        RgbMode::Effect { effect, params, ring } => {
            let settings = RingSettings {
                mode: "effect".into(),
                r: params.color.map_or(255, |c| c.r as i32),
                g: params.color.map_or(0, |c| c.g as i32),
                b: params.color.map_or(128, |c| c.b as i32),
                effect: rgb_convert::effect_display_name(effect).to_string(),
                speed: params.speed as i32,
                direction: rgb_convert::direction_to_index(&params.direction),
                brightness: params.brightness.value() as i32,
            };
            match ring {
                frgb_model::rgb::Ring::Inner => SavedRgbState {
                    ring: 0,
                    inner: settings,
                    outer: RingSettings::default(),
                },
                frgb_model::rgb::Ring::Outer => SavedRgbState {
                    ring: 1,
                    inner: RingSettings::default(),
                    outer: settings,
                },
                frgb_model::rgb::Ring::Both => SavedRgbState {
                    ring: 2,
                    inner: settings.clone(),
                    outer: settings,
                },
            }
        }
        RgbMode::Composed(specs) => {
            // Use the first spec (all fans same in GUI mode).
            if let Some(spec) = specs.first() {
                SavedRgbState {
                    ring: 2, // Source: INFERRED — Composed implies independent zones, default to Both view.
                    inner: ring_settings_from_zone(&spec.inner),
                    outer: ring_settings_from_zone(&spec.outer),
                }
            } else {
                SavedRgbState::default()
            }
        }
        // PerFan/PerLed/TempRgb are edited in their own tabs.
        // Set mode string so the RGB tab can show the active mode name.
        RgbMode::PerFan(_) => SavedRgbState {
            inner: RingSettings {
                mode: "per-fan".into(),
                ..Default::default()
            },
            outer: RingSettings {
                mode: "per-fan".into(),
                ..Default::default()
            },
            ..Default::default()
        },
        RgbMode::PerLed(_) => SavedRgbState {
            inner: RingSettings {
                mode: "per-led".into(),
                ..Default::default()
            },
            outer: RingSettings {
                mode: "per-led".into(),
                ..Default::default()
            },
            ..Default::default()
        },
        RgbMode::TempRgb(_) => SavedRgbState {
            inner: RingSettings {
                mode: "temp-rgb".into(),
                ..Default::default()
            },
            outer: RingSettings {
                mode: "temp-rgb".into(),
                ..Default::default()
            },
            ..Default::default()
        },
        RgbMode::SubZones { .. } => SavedRgbState {
            inner: RingSettings {
                mode: "sub-zones".into(),
                ..Default::default()
            },
            outer: RingSettings {
                mode: "sub-zones".into(),
                ..Default::default()
            },
            ..Default::default()
        },
    }
}

/// Seed the RGB state map from daemon device status.
/// Called on initial connect and reconnect.
pub fn sync_from_status(states: &RgbStateMap, groups: &[frgb_model::config::GroupStatus]) {
    let mut map = states.0.lock().unwrap();
    for gs in groups {
        let state = state_from_rgb_mode(&gs.rgb);
        map.insert(gs.group.id, state);
    }
}

/// Load the currently selected group's state into the UI edit properties.
/// Source: INFERRED — on initial connect and page creation, the UI needs to
/// reflect the actual device state, not default property values.
pub fn load_selected_group(window: &AppWindow, states: &RgbStateMap) {
    let idx = selected_group_index(window);
    if let Some(gid) = group_id_at_index(window, idx) {
        let map = states.0.lock().unwrap();
        let st = map.get(&gid).cloned().unwrap_or_default();
        drop(map);
        window.set_edit_ring(st.ring);
        let active = match st.ring {
            0 => &st.inner,
            1 => &st.outer,
            _ => &st.inner,
        };
        load_ui(window, active);
    }
}

/// Update a single group's state from an RgbChanged event.
pub fn sync_from_event(states: &RgbStateMap, group_id: GroupId, mode: &frgb_model::rgb::RgbMode) {
    let state = state_from_rgb_mode(mode);
    states.0.lock().unwrap().insert(group_id, state);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn group_id_at_index(window: &AppWindow, index: i32) -> Option<GroupId> {
    use slint::Model;
    window
        .get_devices()
        .row_data(index as usize)
        .map(|g| GroupId::new(g.group_id as u8))
}

/// Read current UI edit properties into a RingSettings.
fn capture_ui(window: &AppWindow) -> RingSettings {
    RingSettings {
        mode: window.get_edit_mode().to_string(),
        r: window.get_edit_r(),
        g: window.get_edit_g(),
        b: window.get_edit_b(),
        effect: window.get_edit_effect().to_string(),
        speed: window.get_edit_speed(),
        direction: window.get_edit_direction(),
        brightness: window.get_edit_brightness(),
    }
}

/// Update effect metadata properties from the current effect name.
fn update_effect_metadata(window: &AppWindow, effect_name: &str) {
    if let Some(effect) = rgb_convert::effect_by_name(effect_name) {
        window.set_edit_effect_has_color(effect.supports_color());
        window.set_edit_effect_has_direction(effect.supports_direction());
    } else {
        // Unknown or empty effect — show all controls as safe default
        window.set_edit_effect_has_color(true);
        window.set_edit_effect_has_direction(true);
    }
}

/// Write RingSettings to the UI edit properties.
fn load_ui(window: &AppWindow, s: &RingSettings) {
    window.set_edit_mode(s.mode.as_str().into());
    window.set_edit_r(s.r);
    window.set_edit_g(s.g);
    window.set_edit_b(s.b);
    window.set_edit_effect(s.effect.as_str().into());
    window.set_edit_speed(s.speed);
    window.set_edit_direction(s.direction);
    window.set_edit_brightness(s.brightness);
    update_effect_metadata(window, &s.effect);
}

/// Save current UI into the active ring(s) of a group state.
fn save_to_active_rings(state: &mut SavedRgbState, ui: &RingSettings) {
    match state.ring {
        0 => state.inner = ui.clone(),
        1 => state.outer = ui.clone(),
        _ => {
            state.inner = ui.clone();
            state.outer = ui.clone();
        }
    }
}

/// Build a ZoneSource from RingSettings.
fn zone_source(s: &RingSettings) -> frgb_model::rgb::ZoneSource {
    use frgb_model::rgb::{EffectParams, EffectScope, Rgb, ZoneSource};
    use frgb_model::Brightness;
    match s.mode.as_str() {
        "static" => ZoneSource::Color {
            color: Rgb {
                r: s.r.clamp(0, 255) as u8,
                g: s.g.clamp(0, 255) as u8,
                b: s.b.clamp(0, 255) as u8,
            },
            brightness: Brightness::new(s.brightness.clamp(0, 255) as u8),
        },
        "effect" => {
            if let Some(effect) = rgb_convert::effect_by_name(&s.effect) {
                ZoneSource::Effect {
                    effect,
                    params: EffectParams {
                        speed: s.speed.clamp(1, 5) as u8,
                        direction: rgb_convert::direction_from_index(s.direction),
                        brightness: Brightness::new(s.brightness.clamp(0, 255) as u8),
                        color: Some(Rgb {
                            r: s.r.clamp(0, 255) as u8,
                            g: s.g.clamp(0, 255) as u8,
                            b: s.b.clamp(0, 255) as u8,
                        }),
                        scope: EffectScope::All,
                    },
                }
            } else {
                ZoneSource::Off
            }
        }
        _ => ZoneSource::Off,
    }
}

/// Build an RgbMode::Composed from both ring states.
fn build_mode(state: &SavedRgbState) -> frgb_model::rgb::RgbMode {
    use frgb_model::rgb::{FanZoneSpec, RgbMode, ZoneSource};
    let inner = zone_source(&state.inner);
    let outer = zone_source(&state.outer);
    if matches!(&inner, ZoneSource::Off) && matches!(&outer, ZoneSource::Off) {
        RgbMode::Off
    } else {
        RgbMode::Composed(vec![FanZoneSpec { inner, outer }])
    }
}

/// Capture current UI into the active ring(s), build composed mode, and send.
fn compose_and_send(bridge: &BridgeHandle, window: &AppWindow, states: &StateMap) {
    let idx = selected_group_index(window);
    if let Some(gid) = group_id_at_index(window, idx) {
        let mut map = states.0.lock().unwrap();
        let state = map.entry(gid).or_default();
        state.ring = window.get_edit_ring();
        save_to_active_rings(state, &capture_ui(window));
        let mode = build_mode(state);
        drop(map);
        bridge.send(frgb_ipc::Request::SetRgb { group: gid, mode });
    }
}

// ---------------------------------------------------------------------------
// Wire
// ---------------------------------------------------------------------------

pub fn wire(window: &AppWindow, bridge: &BridgeHandle, states: &RgbStateMap) {
    // Apply to selected group
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        let states = states.clone();
        window.on_rgb_apply(move |_index| {
            if let Some(window) = w.upgrade() {
                compose_and_send(&bridge, &window, &states);
            }
        });
    }

    // Apply to all groups
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        let states = states.clone();
        window.on_rgb_apply_all(move || {
            if let Some(window) = w.upgrade() {
                let ring = window.get_edit_ring();
                let ui = capture_ui(&window);

                let mut template = SavedRgbState {
                    ring,
                    ..Default::default()
                };
                save_to_active_rings(&mut template, &ui);
                let mode = build_mode(&template);

                bridge.send(frgb_ipc::Request::SetRgbAll {
                    target: frgb_ipc::Target::All,
                    mode,
                });

                // Save to every known group
                use slint::Model;
                let model = window.get_devices();
                let mut map = states.0.lock().unwrap();
                for i in 0..model.row_count() {
                    if let Some(g) = model.row_data(i) {
                        let state = map.entry(GroupId::new(g.group_id as u8)).or_default();
                        state.ring = ring;
                        save_to_active_rings(state, &ui);
                    }
                }
            }
        });
    }

    // All off
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        let states = states.clone();
        window.on_rgb_off_all(move || {
            bridge.send(frgb_ipc::Request::SetRgbAll {
                target: frgb_ipc::Target::All,
                mode: frgb_model::rgb::RgbMode::Off,
            });
            if let Some(window) = w.upgrade() {
                use slint::Model;
                let model = window.get_devices();
                let mut map = states.0.lock().unwrap();
                for i in 0..model.row_count() {
                    if let Some(g) = model.row_data(i) {
                        let state = map.entry(GroupId::new(g.group_id as u8)).or_default();
                        state.inner.mode = "off".into();
                        state.outer.mode = "off".into();
                    }
                }
            }
        });
    }

    // Group selected — restore saved state (no hardware send)
    {
        let w = window.as_weak();
        let states = states.clone();
        window.on_rgb_group_selected(move |index| {
            if let Some(window) = w.upgrade() {
                if let Some(gid) = group_id_at_index(&window, index) {
                    let map = states.0.lock().unwrap();
                    let st = map.get(&gid).cloned().unwrap_or_default();
                    drop(map);
                    window.set_edit_ring(st.ring);
                    let active = match st.ring {
                        0 => &st.inner,
                        1 => &st.outer,
                        _ => &st.inner,
                    };
                    load_ui(&window, active);
                }
            }
        });
    }

    // Ring changed — swap per-ring state (no hardware send)
    {
        let w = window.as_weak();
        let states = states.clone();
        window.on_rgb_ring_changed(move |new_ring| {
            if let Some(window) = w.upgrade() {
                let idx = selected_group_index(&window);
                if let Some(gid) = group_id_at_index(&window, idx) {
                    let mut map = states.0.lock().unwrap();
                    let state = map.entry(gid).or_default();
                    save_to_active_rings(state, &capture_ui(&window));
                    state.ring = new_ring;
                    let active = match new_ring {
                        0 => state.inner.clone(),
                        1 => state.outer.clone(),
                        _ => state.inner.clone(),
                    };
                    drop(map);
                    load_ui(&window, &active);
                }
            }
        });
    }

    // Mode changed — explicit pill click, always send
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        let states = states.clone();
        window.on_rgb_mode_changed(move |_mode_str| {
            if let Some(window) = w.upgrade() {
                compose_and_send(&bridge, &window, &states);
            }
        });
    }

    // Color changed — fires on slider released only
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        let states = states.clone();
        window.on_rgb_color_changed(move |_r, _g, _b| {
            if let Some(window) = w.upgrade() {
                if window.get_edit_mode() == "static" {
                    compose_and_send(&bridge, &window, &states);
                }
            }
        });
    }

    // Effect changed — ComboBox selected.
    // Source: INFERRED — Slint ComboBox fires `selected` on creation when the conditional
    // section appears (edit-mode changes to "effect"). Guard: only send if the effect name
    // differs from what's already saved for this group, preventing spurious sends on group select.
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        let states = states.clone();
        window.on_rgb_effect_changed(move |name| {
            if let Some(window) = w.upgrade() {
                update_effect_metadata(&window, &name);
                // Dedup: check if effect actually changed from saved state
                let idx = selected_group_index(&window);
                if let Some(gid) = group_id_at_index(&window, idx) {
                    let map = states.0.lock().unwrap();
                    let current = map.get(&gid).map(|s| match s.ring {
                        0 => s.inner.effect.as_str(),
                        1 => s.outer.effect.as_str(),
                        _ => s.inner.effect.as_str(),
                    });
                    if current == Some(name.as_str()) {
                        return; // Same effect — suppress init-time fire
                    }
                    drop(map);
                }
                compose_and_send(&bridge, &window, &states);
            }
        });
    }

    // Speed changed — fires on slider released only
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        let states = states.clone();
        window.on_rgb_speed_changed(move |_speed| {
            if let Some(window) = w.upgrade() {
                if window.get_edit_mode() == "effect" {
                    compose_and_send(&bridge, &window, &states);
                }
            }
        });
    }

    // Direction changed — explicit pill click, always send
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        let states = states.clone();
        window.on_rgb_direction_changed(move |_dir| {
            if let Some(window) = w.upgrade() {
                if window.get_edit_mode() == "effect" {
                    compose_and_send(&bridge, &window, &states);
                }
            }
        });
    }

    // Brightness changed — fires on slider released only
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        let states = states.clone();
        window.on_rgb_brightness_changed(move |_brt| {
            if let Some(window) = w.upgrade() {
                if window.get_edit_mode() != "off" {
                    compose_and_send(&bridge, &window, &states);
                }
            }
        });
    }
}
