use crate::cli::{Command, DirectionArg, RingArg};
use anyhow::{Context, Result};
use frgb_core::System;
use frgb_lcd::video::FrameSource;
use frgb_model::config::{HwmonChannelConfig, HwmonChannelRole, HwmonCurveExecution};
use frgb_model::device::FanRole;
use frgb_model::effect::Effect;
use frgb_model::ipc::{Request, Response};
use frgb_model::rgb::{EffectParams, EffectScope, FanLedAssignment, FanZoneSpec, Rgb, RgbMode, Ring, ZoneSource};
use frgb_model::speed::SpeedMode;
use frgb_model::Brightness;
use frgb_model::GroupId;

pub fn dispatch(system: &mut System, command: &Command) -> Result<()> {
    match command {
        Command::Status { verbose } => cmd_status(system, *verbose),
        Command::Discover { raw } => cmd_discover(system, *raw),
        Command::Speed { percent, group } => cmd_speed(system, *percent, group.map(GroupId::new)),
        Command::Pwm { group } => cmd_pwm(system, group.map(GroupId::new)),
        Command::Color {
            color,
            group,
            ring,
            inner,
            outer,
            inner_top,
            inner_middle,
            inner_bottom,
            outer_top,
            outer_middle,
            outer_bottom,
            brightness,
        } => cmd_color(
            system,
            color.as_deref(),
            group.map(GroupId::new),
            ring.clone(),
            inner.as_deref(),
            outer.as_deref(),
            SubZoneArgs {
                inner_top: inner_top.as_deref(),
                inner_middle: inner_middle.as_deref(),
                inner_bottom: inner_bottom.as_deref(),
                outer_top: outer_top.as_deref(),
                outer_middle: outer_middle.as_deref(),
                outer_bottom: outer_bottom.as_deref(),
            },
            Brightness::new(*brightness),
        ),
        Command::RgbOff { group } => cmd_rgb_off(system, group.map(GroupId::new)),
        Command::Effect {
            name,
            color,
            group,
            brightness,
            ring,
            speed,
            direction,
        } => cmd_effect(
            system,
            name,
            color,
            group.map(GroupId::new),
            Brightness::new(*brightness),
            ring.clone(),
            *speed,
            direction.clone(),
        ),
        Command::Pump { mode, group } => cmd_pump(system, mode, GroupId::new(*group)),
        Command::Sensors | Command::Play { .. } | Command::Stop { .. } => {
            anyhow::bail!("this command requires the daemon (use without --direct)")
        }
        Command::MbSync { state, group } => {
            let enable = match state.to_lowercase().as_str() {
                "on" | "1" | "true" => true,
                "off" | "0" | "false" => false,
                _ => anyhow::bail!("expected 'on' or 'off', got '{state}'"),
            };
            cmd_mbsync(system, enable, group.map(GroupId::new))
        }
        Command::SetRole { role, group } => {
            let fan_role = parse_role(role)?;
            if let Some(dev) = system.registry.find_by_group_mut(GroupId::new(*group)) {
                dev.role = fan_role;
                println!("Group {group}: role set to {:?}", dev.role);
            } else {
                anyhow::bail!("group {group} not found");
            }
            Ok(())
        }
        Command::Rename { name, group } => {
            if let Some(dev) = system.registry.find_by_group_mut(GroupId::new(*group)) {
                dev.name.clone_from(name);
                println!("Group {group}: renamed to '{name}'");
            } else {
                anyhow::bail!("group {group} not found");
            }
            Ok(())
        }
        Command::Led { color, group, index } => cmd_led(system, color, GroupId::new(*group), *index),
        Command::Bind => cmd_bind(system),
        Command::Unbind { group } => cmd_unbind(system, GroupId::new(*group)),
        Command::Lock => cmd_lock(system),
        Command::Unlock => cmd_unlock(system),
        Command::ListEffects => cmd_list_effects(),
        Command::ListColors => cmd_list_colors(),
        Command::LcdPlay { path, device, fps } => cmd_lcd_play(system, path, *device, *fps),
        Command::LcdCapture {
            display,
            window,
            device,
            fps,
        } => cmd_lcd_capture(system, display, window.as_deref(), *device, *fps),
        Command::LcdGame {
            window,
            launch,
            device,
            fps,
        } => cmd_lcd_game(system, window, launch.as_deref(), *device, *fps),
        Command::LcdH264 { path, device } => cmd_lcd_h264(system, path, *device),
        Command::Mobo { action } => match action {
            crate::cli::MoboAction::Name { pwm, name, role, model, min } => {
                cmd_mobo_name(*pwm, name.clone(), role.clone(), model.clone(), *min)
            }
        },
    }
}

// ANSI color helpers
const BLUE: &str = "\x1b[34m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn parse_role(s: &str) -> Result<FanRole> {
    match s.to_lowercase().as_str() {
        "intake" | "in" => Ok(FanRole::Intake),
        "exhaust" | "out" => Ok(FanRole::Exhaust),
        "pump" => Ok(FanRole::Pump),
        other => Ok(FanRole::Custom(other.to_string())),
    }
}

pub fn parse_hwmon_role(s: &str) -> Result<HwmonChannelRole> {
    match s.to_lowercase().as_str() {
        "intake" => Ok(HwmonChannelRole::Intake),
        "exhaust" => Ok(HwmonChannelRole::Exhaust),
        "pump" => Ok(HwmonChannelRole::Pump),
        "fan" => Ok(HwmonChannelRole::Fan),
        other => anyhow::bail!(
            "unknown role '{other}' — must be intake, exhaust, pump, or fan"
        ),
    }
}

pub fn upsert_hwmon_channel(
    cfg: &mut frgb_model::config::Config,
    channel: HwmonChannelConfig,
) {
    if let Some(existing) = cfg.hwmon.channels.iter_mut().find(|c| c.pwm == channel.pwm) {
        *existing = channel;
    } else {
        cfg.hwmon.channels.push(channel);
    }
}

pub fn format_hwmon_managed_row(
    pwm: u8,
    name: &str,
    role: HwmonChannelRole,
    rpm: u16,
    pwm_byte: u8,
    mode: &str,
) -> String {
    let pct = (pwm_byte as u16 * 100 + 127) / 255;
    let role_name = match role {
        HwmonChannelRole::Intake => "intake",
        HwmonChannelRole::Exhaust => "exhaust",
        HwmonChannelRole::Pump => "pump",
        HwmonChannelRole::Fan => "fan",
    };
    format!(
        "  pwm{pwm:<2} {name:<24} {rpm:>5} RPM {pct:>3}%  {role_name:<7} {mode}",
        pwm = pwm, name = name, rpm = rpm, pct = pct, role_name = role_name, mode = mode,
    )
}

/// Map a raw `pwmN_enable` value to a human-readable mode label.
pub fn hwmon_mode_label(enable: Option<u8>) -> String {
    match enable {
        Some(0) => "off".to_string(),
        Some(1) => "manual".to_string(),
        Some(2..=4) => "auto".to_string(),
        Some(5) => "smart".to_string(),
        Some(n) => format!("mode:{n}"),
        None => "?".to_string(),
    }
}

pub fn format_hwmon_unnamed_hint(unconfigured: &[u8]) -> String {
    let list: Vec<String> = unconfigured.iter().map(|p| format!("pwm{p}")).collect();
    format!(
        "  {} detected but unnamed: {}\n  Run: ./r mobo name <pwm> <label> [--role R]",
        unconfigured.len(),
        list.join(", "),
    )
}

fn cmd_mobo_name(
    pwm: u8,
    name: String,
    role: String,
    model: Option<String>,
    min: Option<u8>,
) -> Result<()> {
    let parsed_role = parse_hwmon_role(&role)?;
    let min_pwm = min.map(|p| frgb_core::hwmon_backend::writer::pct_to_byte(p, 0)).unwrap_or(0);

    let mut cfg = frgb_core::config::load_config().context("load config")?;
    upsert_hwmon_channel(
        &mut cfg,
        HwmonChannelConfig {
            pwm,
            name: name.clone(),
            role: parsed_role,
            model,
            min_pwm,
            curve_execution: HwmonCurveExecution::default(),
        },
    );
    frgb_core::config::save_config(&cfg).context("save config")?;
    println!("hwmon: pwm{pwm} named '{name}' (role={role})");
    Ok(())
}

fn cmd_status(system: &System, verbose: bool) -> Result<()> {
    if system.devices().is_empty() {
        println!("No fan groups discovered.");
        return Ok(());
    }
    let mut devices: Vec<_> = system.devices().iter().collect();
    devices.sort_by_key(|d| d.group);

    let mut total_intake_cfm: f32 = 0.0;
    let mut total_exhaust_cfm: f32 = 0.0;

    // AURA channels are RGB-only, not fans — render distinctly so they're not
    // shown with bogus "Unknown / 0 RPM / PWM" attributes.
    let aura_backend = system
        .backends()
        .iter()
        .find(|b| b.name() == "aura")
        .and_then(|b| b.as_any().downcast_ref::<frgb_core::AuraBackend>());

    for device in &devices {
        if matches!(device.device_type, frgb_model::device::DeviceType::Aura) {
            if !verbose {
                continue;
            }
            let led_count = aura_backend
                .map(|a| a.led_count_for_group(device.group))
                .unwrap_or(0);
            let state_str = match device.state.rgb_mode {
                Some(_) => "managed".to_string(),
                None => "unset".to_string(),
            };
            println!(
                "Group {}: {DIM}AURA RGB{RESET} — {} LEDs ({state_str})",
                device.group, led_count,
            );
            continue;
        }

        let detail = if device.device_type.is_fan() {
            format_fans_type_detail(&device.fans_type(), &system.specs)
        } else {
            format!("{:?}", device.device_type)
        };

        let rpms: Vec<u16> = device
            .fans_rpm()
            .iter()
            .take(device.fan_count() as usize)
            .copied()
            .collect();
        let rpm_str = if rpms.is_empty() {
            "n/a".to_string()
        } else {
            rpms.iter().map(|rpm| format!("{rpm}")).collect::<Vec<_>>().join("/")
        };

        // Speed %: use explicit value if set, otherwise derive from RPM/max_rpm
        let spec = device
            .slots
            .first()
            .and_then(|s| system.specs.lookup_fans_type(s.fans_type));
        let max_rpm = spec.and_then(|s| s.max_rpm).unwrap_or(0) as f32;

        let speed_pct: Option<f32> = match device.state.speed_percent {
            Some(pct) => Some(pct.value() as f32),
            None => {
                // Only fans have a meaningful RPM→% derivation: they scale
                // linearly from ~0 RPM at 0% to max_rpm at 100%. Pumps have
                // a non-zero minimum (1600 RPM) and — in direct mode where
                // commanded state isn't preserved across CLI invocations —
                // we can't tell whether the measured RPM reflects a live
                // command or a decayed idle. Showing a derived percent in
                // that case invents information we don't have.
                if matches!(device.role, FanRole::Pump) {
                    None
                } else if max_rpm > 0.0 && !rpms.is_empty() {
                    let avg_rpm: f32 = rpms.iter().map(|&r| r as f32).sum::<f32>() / rpms.len() as f32;
                    Some((avg_rpm / max_rpm * 100.0).min(100.0))
                } else {
                    None
                }
            }
        };

        let (speed_str, group_cfm) = if device.mb_sync {
            (format!("{YELLOW}MB{RESET}"), None)
        } else {
            let s = match speed_pct {
                // Pumps take 15-30s of mechanical spin-up to reach the
                // commanded RPM. During that window the RPM column is
                // mid-spinup and the percent is the target — show them as
                // distinct quantities with an arrow instead of reading them
                // as a single "current speed" value.
                Some(pct) if matches!(device.role, FanRole::Pump) => {
                    format!("{DIM}→{RESET} {pct:.0}%")
                }
                Some(pct) => format!("{pct:.0}%"),
                None if matches!(device.role, FanRole::Pump) => format!("{DIM}—{RESET}"),
                None => "PWM".to_string(),
            };
            let cfm = spec
                .and_then(|sp| sp.cfm)
                .and_then(|cfm_per_fan| speed_pct.map(|pct| cfm_per_fan * (pct / 100.0) * device.fan_count() as f32));
            (s, cfm)
        };

        let cfm_str = group_cfm.map(|cfm| format!("{cfm:.0} CFM")).unwrap_or_default();

        // Track intake/exhaust totals
        if let Some(cfm) = group_cfm {
            match &device.role {
                FanRole::Intake => total_intake_cfm += cfm,
                FanRole::Exhaust => total_exhaust_cfm += cfm,
                _ => {}
            }
        }

        // Colorize role
        let (role_color, role_str) = match &device.role {
            FanRole::Intake => (BLUE, "↓ in"),
            FanRole::Exhaust => (RED, "↑ out"),
            FanRole::Pump => (DIM, "⟳ pump"),
            FanRole::Custom(s) => (DIM, s.as_str()),
        };

        println!(
            "Group {}: {} ({}{}{}) — {} RPM  {}  {}{}{}",
            device.group, detail, role_color, role_str, RESET, rpm_str, speed_str, role_color, cfm_str, RESET,
        );

        if verbose {
            for (i, mac) in device.mac_ids.iter().enumerate() {
                println!("  Fan {}: {}", i + 1, mac.to_hex());
            }
            println!("  TX ref: {}", device.tx_ref.to_hex());
            if let Some(ref rgb) = device.state.rgb_mode {
                println!("  RGB: {:?}", rgb);
            }
        }
    }

    // CFM summary
    if total_intake_cfm > 0.0 || total_exhaust_cfm > 0.0 {
        println!();
        println!(
            "Airflow: {BLUE}↓ {:.0} CFM in{RESET}  {RED}↑ {:.0} CFM out{RESET}  net {:.0} CFM",
            total_intake_cfm,
            total_exhaust_cfm,
            total_intake_cfm - total_exhaust_cfm,
        );
        if total_exhaust_cfm > total_intake_cfm {
            println!("{YELLOW}⚠ Negative pressure — exhaust exceeds intake{RESET}");
        }
    }

    // Hwmon motherboard fans section (Task 11)
    if let Some(hwmon_backend) = system
        .backends()
        .iter()
        .find(|b| b.name() == "hwmon")
        .and_then(|b| b.as_any().downcast_ref::<frgb_core::HwmonBackend>())
    {
        if hwmon_backend.channel_count() > 0 {
            println!();
            println!("Motherboard fans ({}):", hwmon_backend.chip_name());
            let cfg = frgb_core::config::load_config().unwrap_or_default();
            let mut records: Vec<&frgb_core::DiscoveredDevice> = system
                .raw_records
                .iter()
                .filter(|r| r.dev_type == frgb_core::hwmon_backend::DEV_TYPE_HWMON)
                .collect();
            records.sort_by_key(|r| r.channel);
            for rec in records {
                if let Some(ch_cfg) = cfg.hwmon.channels.iter().find(|c| c.pwm == rec.channel) {
                    println!(
                        "{}",
                        format_hwmon_managed_row(
                            rec.channel,
                            &ch_cfg.name,
                            ch_cfg.role,
                            rec.fans_rpm[0],
                            rec.fans_pwm[0],
                            &hwmon_mode_label(hwmon_backend.current_enable(rec.channel)),
                        )
                    );
                }
            }
        }
        if !hwmon_backend.unconfigured_channels().is_empty() {
            println!();
            println!("{}", format_hwmon_unnamed_hint(hwmon_backend.unconfigured_channels()));
        }
    }

    Ok(())
}

fn cmd_discover(system: &System, raw: bool) -> Result<()> {
    if raw {
        println!("Raw device records ({}):", system.raw_records.len());
        for rec in &system.raw_records {
            let master_str = if rec.master == frgb_model::device::DeviceId::ZERO {
                "none".to_string()
            } else {
                rec.master.to_hex()
            };
            println!(
                "  mac={} master={} group={} dev_type=0x{:02X} fan_num={} fans_type={:?} rpm={:?} pwm={:?} cmd_seq={}",
                rec.id.to_hex(),
                master_str,
                rec.group,
                rec.dev_type,
                rec.fan_count,
                rec.fans_type,
                rec.fans_rpm,
                rec.fans_pwm,
                rec.cmd_seq,
            );
        }
        println!();
    }

    let mut devices: Vec<_> = system.devices().iter().collect();
    devices.sort_by_key(|d| d.group);

    println!("Discovered {} fan group(s):", devices.len());
    for device in &devices {
        let detail = if device.device_type.is_fan() {
            format_fans_type_detail(&device.fans_type(), &system.specs)
        } else {
            format!("{:?}", device.device_type)
        };
        println!(
            "  Group {}: [{}] {}",
            device.group,
            device
                .mac_ids
                .iter()
                .map(|id| id.to_hex())
                .collect::<Vec<_>>()
                .join(", "),
            detail,
        );
    }

    if !system.unbound.is_empty() {
        println!("\nUnbound devices on channel:");
        for dev in &system.unbound {
            let detail = if dev.device_type.is_fan() {
                format_fans_type_detail(&dev.fans_type, &system.specs)
            } else {
                format!("{:?}", dev.device_type)
            };
            let master_str = if dev.master == frgb_model::device::DeviceId::ZERO {
                "none".to_string()
            } else {
                dev.master.to_hex()
            };
            println!(
                "  {}  {}  master={}  group={}",
                dev.mac.to_hex(),
                detail,
                master_str,
                dev.group,
            );
        }
    }

    Ok(())
}

/// Format fans_type slots into a human-readable breakdown using DeviceSpec names.
fn format_fans_type_detail(fans_type: &[u8; 4], specs: &frgb_model::spec::SpecRegistry) -> String {
    let slots: Vec<String> = fans_type
        .iter()
        .filter(|&&ft| ft != 0)
        .map(|&ft| {
            specs
                .lookup_fans_type(ft)
                .map(|spec| spec.name.clone())
                .unwrap_or_else(|| format!("Unknown({})", ft))
        })
        .collect();

    if slots.is_empty() {
        return "no fans".to_string();
    }

    // Group consecutive same-name slots
    let mut parts: Vec<(usize, &str)> = Vec::new();
    for name in &slots {
        if let Some(last) = parts.last_mut() {
            if last.1 == name.as_str() {
                last.0 += 1;
                continue;
            }
        }
        parts.push((1, name.as_str()));
    }

    parts
        .iter()
        .map(|(count, name)| format!("{count} × {name}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// After mutating hardware speed state in direct mode, show the user what the
/// hardware actually did rather than echoing back the commanded value.
///
/// In direct mode the CLI has no long-lived state: `state.speed_percent` is
/// set by `system.set_speed` but the process exits immediately afterward, so
/// a subsequent `./r status` invocation can't see it. More importantly, for
/// device families that don't expose a meaningful PWM byte (CL fans report
/// 0x06 regardless; AIO pumps don't have fans_pwm at all), the cached value
/// is meaningless anyway. Clearing the cached state and re-displaying via the
/// RPM/max_rpm fallback gives real feedback from the hardware's response.
fn refresh_and_show_status(system: &mut System, affected: &[GroupId]) -> Result<()> {
    // Clear the just-written cache so cmd_status falls through to the
    // RPM-derived display for the affected groups.
    // Skip pumps (AIO or standalone WaterBlock): they don't report speed via
    // PWM bytes, so clearing would show an inaccurate RPM-derived percentage
    // that varies with hardware tolerances rather than the commanded value.
    for gid in affected {
        let is_pump = system
            .find_group(*gid)
            .is_ok_and(|d| d.device_type.is_aio() || matches!(d.role, frgb_model::device::FanRole::Pump));
        if !is_pump {
            system.registry.update_state(*gid, |s| s.speed_percent = None);
        }
    }
    // Give fans a moment to respond before re-reading RPMs. The RF poll loop
    // in the receiver updates its reported fans_speed every few hundred ms.
    // Pumps take 15-30s of mechanical spin-up to reach commanded RPM — much
    // too long to block an interactive command on. Instead, cmd_status shows
    // pump rows with an explicit `→ target` arrow so the current (mid-spinup)
    // RPM and the commanded percent are clearly distinct.
    std::thread::sleep(std::time::Duration::from_millis(800));
    let _ = system.discover();
    cmd_status(system, false)
}

fn cmd_speed(system: &mut System, percent: u8, group: Option<GroupId>) -> Result<()> {
    let mode = SpeedMode::Manual(frgb_model::SpeedPercent::new(percent));
    let affected: Vec<GroupId> = if let Some(gid) = group {
        system
            .set_speed(gid, &mode)
            .context(format!("Failed to set speed for group {gid}"))?;
        vec![gid]
    } else {
        let groups = system.group_ids();
        let mut changed = Vec::new();
        for gid in &groups {
            if !system.is_fan_capable(*gid) {
                continue;
            }
            let Ok(dev) = system.find_group(*gid) else { continue };
            if dev.mb_sync {
                println!("Group {gid}: skipped (motherboard control)");
                continue;
            }
            system
                .set_speed(*gid, &mode)
                .context(format!("Failed to set speed for group {gid}"))?;
            changed.push(*gid);
        }
        changed
    };
    refresh_and_show_status(system, &affected)
}

fn cmd_pwm(system: &mut System, group: Option<GroupId>) -> Result<()> {
    let mode = SpeedMode::Pwm;
    let affected: Vec<GroupId> = if let Some(gid) = group {
        system
            .set_speed(gid, &mode)
            .context(format!("Failed to set PWM for group {gid}"))?;
        vec![gid]
    } else {
        let groups = system.group_ids();
        let mut changed = Vec::new();
        for gid in &groups {
            if !system.is_fan_capable(*gid) {
                continue;
            }
            system
                .set_speed(*gid, &mode)
                .context(format!("Failed to set PWM for group {gid}"))?;
            changed.push(*gid);
        }
        changed
    };
    refresh_and_show_status(system, &affected)
}

fn parse_pump_mode(s: &str) -> Result<frgb_model::speed::PumpMode> {
    use frgb_model::speed::PumpMode;
    match s.to_lowercase().as_str() {
        "quiet" => Ok(PumpMode::Quiet),
        "standard" => Ok(PumpMode::Standard),
        "high" => Ok(PumpMode::High),
        "full" => Ok(PumpMode::Full),
        other => {
            let pct: u8 = other
                .parse()
                .context(format!("expected quiet/standard/high/full or 0-100, got '{other}'"))?;
            Ok(PumpMode::Fixed(pct))
        }
    }
}

fn cmd_pump(system: &mut System, mode_str: &str, group: GroupId) -> Result<()> {
    let mode = parse_pump_mode(mode_str)?;
    system
        .set_pump(group, &mode)
        .context(format!("Failed to set pump mode for group {group}"))?;
    refresh_and_show_status(system, &[group])
}

fn cmd_led(system: &mut System, color_str: &str, group: GroupId, led_index: usize) -> Result<()> {
    use frgb_rgb::layout::LedLayout;

    let device = system.find_group(group)?;
    let layout = LedLayout::for_device(device.device_type);
    let fan_count = device.fan_count() as usize;
    let total = layout.total_per_fan as usize;
    let inner_n = layout.inner_count as usize;
    let outer_n = layout.outer_count as usize;

    if total == 0 {
        anyhow::bail!("device type {:?} has no addressable LEDs", device.device_type);
    }
    if led_index >= total {
        anyhow::bail!("LED index {led_index} out of range (max {})", total - 1);
    }

    let colors: Vec<&str> = color_str.split(',').collect();
    let black = Rgb { r: 0, g: 0, b: 0 };

    let (is_inner, off) = frgb_core::services::rgb::per_led_zone_offset(device.device_type, led_index);

    let mut assignments = Vec::with_capacity(fan_count);
    for fan in 0..fan_count {
        let color = parse_color(colors.get(fan).copied().unwrap_or(colors[0])).map_err(|e| anyhow::anyhow!(e))?;
        let mut inner = vec![black; inner_n];
        let mut outer = vec![black; outer_n];
        if is_inner {
            if off < inner_n {
                inner[off] = color;
            }
        } else if off < outer_n {
            outer[off] = color;
        }
        assignments.push(FanLedAssignment { inner, outer });
    }

    let mode = RgbMode::PerLed(assignments);
    system
        .set_rgb(group, &mode)
        .context(format!("Failed to set LED for group {group}"))?;
    println!("Group {group}: LED {led_index} set");
    Ok(())
}

/// Bundle of TL sub-zone color args, kept together for signature ergonomics.
#[derive(Clone, Copy, Default)]
pub(crate) struct SubZoneArgs<'a> {
    pub inner_top: Option<&'a str>,
    pub inner_middle: Option<&'a str>,
    pub inner_bottom: Option<&'a str>,
    pub outer_top: Option<&'a str>,
    pub outer_middle: Option<&'a str>,
    pub outer_bottom: Option<&'a str>,
}

impl SubZoneArgs<'_> {
    pub(crate) fn any(&self) -> bool {
        self.inner_top.is_some()
            || self.inner_middle.is_some()
            || self.inner_bottom.is_some()
            || self.outer_top.is_some()
            || self.outer_middle.is_some()
            || self.outer_bottom.is_some()
    }
}

fn cmd_color(
    system: &mut System,
    color_str: Option<&str>,
    group: Option<GroupId>,
    ring_arg: RingArg,
    inner_str: Option<&str>,
    outer_str: Option<&str>,
    sub_zones: SubZoneArgs<'_>,
    brightness: Brightness,
) -> Result<()> {
    if (inner_str.is_some() || outer_str.is_some()) && !matches!(ring_arg, RingArg::Both) {
        anyhow::bail!("--ring cannot be combined with --inner/--outer");
    }
    let mode = build_color_mode(color_str, ring_arg, inner_str, outer_str, sub_zones, brightness)?;
    apply_rgb(system, group, &mode, "color applied")
}

fn cmd_rgb_off(system: &mut System, group: Option<GroupId>) -> Result<()> {
    apply_rgb(system, group, &RgbMode::Off, "RGB off")
}

/// Apply an RGB mode to one or all groups.
fn apply_rgb(system: &mut System, group: Option<GroupId>, mode: &RgbMode, label: &str) -> Result<()> {
    if let Some(gid) = group {
        system
            .set_rgb(gid, mode)
            .context(format!("Failed to set RGB for group {gid}"))?;
        println!("Group {gid}: {label}");
    } else {
        let groups = system.group_ids();
        for gid in groups {
            system
                .set_rgb(gid, mode)
                .context(format!("Failed to set RGB for group {gid}"))?;
        }
        println!("All groups: {label}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_effect(
    system: &mut System,
    name: &str,
    color_str: &str,
    group: Option<GroupId>,
    brightness: Brightness,
    ring_arg: RingArg,
    speed: u8,
    direction_arg: DirectionArg,
) -> Result<()> {
    let mode = build_effect_mode(name, color_str, ring_arg, speed, direction_arg, brightness)?;
    apply_rgb(system, group, &mode, name)
}

fn cmd_bind(system: &mut System) -> Result<()> {
    if system.unbound.is_empty() {
        println!("No unbound devices found.");
        return Ok(());
    }

    println!("Unbound devices:");
    for (i, dev) in system.unbound.iter().enumerate() {
        let detail = format_fans_type_detail(&dev.fans_type, &system.specs);
        let master_str = if dev.master == frgb_model::device::DeviceId::ZERO {
            "none".to_string()
        } else {
            dev.master.to_hex()
        };
        println!("  {}) {}  {}  master={}", i + 1, dev.mac.to_hex(), detail, master_str,);
    }

    let device_idx = if system.unbound.len() == 1 {
        println!("Auto-selecting device 1 (only one unbound)");
        1
    } else {
        print!("Select device [1-{}]: ", system.unbound.len());
        read_usize_default(1)?
    };
    if device_idx < 1 || device_idx > system.unbound.len() {
        anyhow::bail!("Invalid selection: {device_idx}");
    }
    let device = system.unbound[device_idx - 1].clone();

    if device.master != frgb_model::device::DeviceId::ZERO {
        print!(
            "Warning: this device is bound to another controller (master={}).\n\
             Binding may fail if the other controller has it locked.\n\
             Proceed? [y/N]: ",
            device.master.to_hex()
        );
        let answer = read_line()?.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            println!("Cancelled.");
            return Ok(());
        }
    }

    let used = system.group_ids();
    let available: Vec<GroupId> = (1..=8u8).map(GroupId::new).filter(|g| !used.contains(g)).collect();
    if available.is_empty() {
        anyhow::bail!("All groups 1-8 are in use");
    }
    let avail_str: Vec<String> = available.iter().map(|g| g.to_string()).collect();
    print!("Assign to group [{}]: ", avail_str.join(", "));
    let group_num: u8 = read_line()?
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid group number"))?;
    let group = GroupId::new(group_num);
    if !available.contains(&group) {
        anyhow::bail!("Group {} is already in use. Available: {}", group, avail_str.join(", "));
    }

    println!("Binding {}...", device.mac.to_hex());
    let rf = system
        .rf_ext()
        .ok_or_else(|| anyhow::anyhow!("Backend does not support bind operation"))?;
    rf.bind_device(&device.mac, group)
        .context("Failed to send bind command")?;

    println!("Verifying...");
    std::thread::sleep(std::time::Duration::from_secs(2));

    system.discover().context("Verification discovery failed")?;

    let found = system
        .devices()
        .iter()
        .any(|d| d.group == group && d.mac_ids.contains(&device.mac));
    let still_unbound = system.unbound.iter().any(|u| u.mac == device.mac);

    if found {
        println!("Bound {} to group {} (verified, locked)", device.mac.to_hex(), group);
    } else if still_unbound {
        println!("Bind failed — device still unbound (master=none, group=254).");
        println!("The lock may not have reached the device.");
    } else {
        println!("Device not found in group {} or unbound list.", group);
        println!("Run `frgb discover --raw` to check device state.");
    }

    Ok(())
}

fn cmd_unbind(system: &mut System, group: GroupId) -> Result<()> {
    let device = system.find_group(group)?;
    let mac = *device
        .mac_ids
        .first()
        .ok_or_else(|| anyhow::anyhow!("group {group} has no device MAC"))?;
    let rf = system
        .rf_ext()
        .ok_or_else(|| anyhow::anyhow!("Backend does not support unbind"))?;
    rf.unbind_device(&mac, group)
        .context(format!("Failed to unbind group {group}"))?;
    println!("Group {group}: unbound ({})", mac.to_hex());
    Ok(())
}

fn cmd_mbsync(system: &mut System, enable: bool, group: Option<GroupId>) -> Result<()> {
    let label = if enable { "motherboard control" } else { "frgb control" };
    if let Some(gid) = group {
        system
            .set_mb_sync(gid, enable, None)
            .context(format!("Failed to set MB sync for group {gid}"))?;
        println!("Group {gid}: {label}");
    } else {
        let groups = system.group_ids();
        for gid in groups {
            system
                .set_mb_sync(gid, enable, None)
                .context(format!("Failed to set MB sync for group {gid}"))?;
        }
        println!("All groups: {label}");
    }
    Ok(())
}

fn cmd_lock(system: &System) -> Result<()> {
    let rf = system
        .rf_ext()
        .ok_or_else(|| anyhow::anyhow!("Backend does not support lock operation"))?;
    rf.lock().context("Failed to lock devices")?;
    println!("All devices: locked");
    Ok(())
}

fn cmd_unlock(system: &System) -> Result<()> {
    let rf = system
        .rf_ext()
        .ok_or_else(|| anyhow::anyhow!("Backend does not support unlock operation"))?;
    rf.unlock().context("Failed to unlock devices")?;
    println!("All devices: unlocked");
    Ok(())
}

fn read_line() -> Result<String> {
    use std::io::{self, BufRead, Write};
    io::stdout().flush()?;
    let mut line = String::new();
    let bytes_read = io::stdin().lock().read_line(&mut line)?;
    if bytes_read == 0 {
        anyhow::bail!("stdin closed (EOF) — interactive input required");
    }
    Ok(line)
}

fn read_usize_default(default: usize) -> Result<usize> {
    let line = read_line()?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(default);
    }
    trimmed
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("Invalid number: {trimmed}"))
}

fn parse_color(s: &str) -> std::result::Result<Rgb, String> {
    if let Some(rgb) = Rgb::from_name(s) {
        return Ok(rgb);
    }
    let hex = s.strip_prefix('#').unwrap_or(s);
    Rgb::from_hex(hex)
}

// ---------------------------------------------------------------------------
// Mode builders — shared between direct dispatch and IPC bridge
// ---------------------------------------------------------------------------

fn build_color_mode(
    color_str: Option<&str>,
    ring_arg: RingArg,
    inner_str: Option<&str>,
    outer_str: Option<&str>,
    sub_zones: SubZoneArgs<'_>,
    brightness: Brightness,
) -> Result<RgbMode> {
    if sub_zones.any() {
        if color_str.is_some() || inner_str.is_some() || outer_str.is_some() {
            anyhow::bail!("sub-zone flags cannot be combined with positional color or --inner/--outer");
        }
        let parse = |s: Option<&str>| -> Result<Option<Rgb>> {
            match s {
                Some(s) => Ok(Some(parse_color(s).map_err(|e| anyhow::anyhow!("{e}"))?)),
                None => Ok(None),
            }
        };
        return Ok(RgbMode::SubZones {
            inner_top: parse(sub_zones.inner_top)?,
            inner_middle: parse(sub_zones.inner_middle)?,
            inner_bottom: parse(sub_zones.inner_bottom)?,
            outer_top: parse(sub_zones.outer_top)?,
            outer_middle: parse(sub_zones.outer_middle)?,
            outer_bottom: parse(sub_zones.outer_bottom)?,
            brightness,
        });
    }
    if inner_str.is_some() || outer_str.is_some() {
        let inner = match inner_str {
            Some(s) => ZoneSource::Color {
                color: parse_color(s).map_err(|e| anyhow::anyhow!("{e}"))?,
                brightness,
            },
            None => ZoneSource::Off,
        };
        let outer = match outer_str {
            Some(s) => ZoneSource::Color {
                color: parse_color(s).map_err(|e| anyhow::anyhow!("{e}"))?,
                brightness,
            },
            None => ZoneSource::Off,
        };
        Ok(RgbMode::Composed(vec![FanZoneSpec { inner, outer }]))
    } else if let Some(color_str) = color_str {
        if color_str.contains(',') {
            let specs: Vec<FanZoneSpec> = color_str
                .split(',')
                .map(|s| {
                    let color = parse_color(s.trim()).map_err(|e| anyhow::anyhow!("{e}"))?;
                    let src = ZoneSource::Color { color, brightness };
                    let ring: Ring = ring_arg.clone().into();
                    Ok(FanZoneSpec::from_ring(ring, src))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(RgbMode::Composed(specs))
        } else {
            let color = parse_color(color_str)
                .map_err(|e| anyhow::anyhow!("{e}"))
                .context(format!("Invalid color: '{color_str}'"))?;
            let ring: Ring = ring_arg.into();
            Ok(RgbMode::Static {
                ring,
                color,
                brightness,
            })
        }
    } else {
        anyhow::bail!("Provide a color, or use --inner/--outer for per-ring colors")
    }
}

fn build_effect_mode(
    name: &str,
    color_str: &str,
    ring_arg: RingArg,
    speed: u8,
    direction_arg: DirectionArg,
    brightness: Brightness,
) -> Result<RgbMode> {
    let effect = Effect::from_name(name).ok_or_else(|| anyhow::anyhow!("Unknown effect: {name}"))?;
    let color = parse_color(color_str).map_err(|e| anyhow::anyhow!("{e}"))?;
    let params = EffectParams {
        speed: speed.clamp(1, 5),
        direction: direction_arg.into(),
        brightness,
        color: Some(color),
        scope: EffectScope::All,
    };
    let ring: Ring = ring_arg.into();
    Ok(RgbMode::Effect { effect, params, ring })
}

/// Build a SetRgb or SetRgbAll request depending on whether a group is specified.
fn rgb_request(group: Option<GroupId>, mode: RgbMode) -> Request {
    if let Some(gid) = group {
        Request::SetRgb { group: gid, mode }
    } else {
        Request::SetRgbAll {
            target: frgb_model::ipc::Target::All,
            mode,
        }
    }
}

// ---------------------------------------------------------------------------
// Pure lookup commands — no USB or daemon access required
// ---------------------------------------------------------------------------

pub fn cmd_list_effects() -> Result<()> {
    println!("{:<22} {:<7} {:<9}", "Effect", "Color?", "Direction?");
    println!("{}", "-".repeat(40));
    for effect in frgb_model::effect::Effect::all() {
        let color = if effect.supports_color() { "yes" } else { "no" };
        let dir = if effect.supports_direction() { "yes" } else { "no" };
        println!("{:<22} {:<7} {:<9}", effect.name(), color, dir);
    }
    Ok(())
}

pub fn cmd_list_colors() -> Result<()> {
    println!("{:<10} RGB (hex)", "Name");
    println!("{}", "-".repeat(25));
    let colors: &[(&str, u8, u8, u8)] = &[
        ("red", 254, 0, 0),
        ("orange", 254, 60, 0),
        ("yellow", 254, 160, 0),
        ("green", 0, 254, 0),
        ("cyan", 0, 254, 254),
        ("blue", 0, 0, 254),
        ("purple", 127, 0, 254),
        ("pink", 254, 0, 254),
        ("magenta", 254, 0, 254),
        ("white", 254, 254, 254),
        ("black", 0, 0, 0),
        ("off", 0, 0, 0),
    ];
    for &(name, r, g, b) in colors {
        println!("{:<10} {:02x}{:02x}{:02x}", name, r, g, b);
    }
    println!();
    println!("Hex format also accepted: frgb color ff4400 -g 1");
    Ok(())
}

// ---------------------------------------------------------------------------
// LCD streaming
// ---------------------------------------------------------------------------

fn cmd_lcd_play(system: &System, path: &str, device_idx: u8, fps: u8) -> Result<()> {
    let lcd_ids = system.lcd_device_ids();
    let device_id = lcd_ids.get(device_idx as usize).ok_or_else(|| {
        anyhow::anyhow!(
            "LCD device index {} not found ({} available)",
            device_idx,
            lcd_ids.len()
        )
    })?;

    let lcd_info = system.lcd_device_info();
    let info = lcd_info
        .get(device_idx as usize)
        .ok_or_else(|| anyhow::anyhow!("LCD device info not found for index {}", device_idx))?;
    let (width, height) = (info.width, info.height);

    println!(
        "Streaming {} to LCD {} ({}x{} @ {}fps)...",
        path, device_idx, width, height, fps
    );
    println!("Press Ctrl+C to stop.");

    let mut source = frgb_lcd::video::FfmpegFrameSource::new(path, width, height, fps as u32, 5)
        .map_err(|e| anyhow::anyhow!("failed to start ffmpeg: {e}"))?;

    let lcd = system
        .lcd_ext()
        .ok_or_else(|| anyhow::anyhow!("no LCD backend available"))?;

    let interval = source.frame_interval();
    let mut frame_count = 0u64;

    while let Some(jpeg) = source.next_frame() {
        let start = std::time::Instant::now();
        lcd.send_frame(device_id, &jpeg).context("LCD frame send failed")?;
        frame_count += 1;

        let elapsed = start.elapsed();
        if elapsed < interval {
            std::thread::sleep(interval - elapsed);
        }

        if frame_count.is_multiple_of(100) {
            eprintln!("  {} frames sent", frame_count);
        }
    }

    println!("Done — {} frames streamed.", frame_count);
    Ok(())
}

fn cmd_lcd_capture(system: &System, display: &str, window: Option<&str>, device: u8, fps: u8) -> Result<()> {
    let lcd_ids = system.lcd_device_ids();
    let device_id = lcd_ids
        .get(device as usize)
        .ok_or_else(|| anyhow::anyhow!("LCD device index {} not found ({} available)", device, lcd_ids.len()))?;

    let lcd_info = system.lcd_device_info();
    let info = lcd_info
        .get(device as usize)
        .ok_or_else(|| anyhow::anyhow!("LCD device info not found for index {}", device))?;
    let (width, height) = (info.width, info.height);

    println!(
        "Capturing {} → LCD {} ({}x{}, {}fps)",
        window.unwrap_or("screen"),
        device,
        width,
        height,
        fps
    );
    println!("Press Ctrl+C to stop");

    let mut source = frgb_lcd::capture::ScreenCaptureSource::new(display, window, width, height, fps as u32)
        .map_err(|e| anyhow::anyhow!("failed to start screen capture: {e}"))?;

    let lcd = system
        .lcd_ext()
        .ok_or_else(|| anyhow::anyhow!("no LCD backend available"))?;

    let interval = source.frame_interval();
    let mut frame_count = 0u64;

    while let Some(jpeg) = source.next_frame() {
        let start = std::time::Instant::now();
        lcd.send_frame(device_id, &jpeg).context("LCD frame send failed")?;
        frame_count += 1;

        let elapsed = start.elapsed();
        if elapsed < interval {
            std::thread::sleep(interval - elapsed);
        }

        if frame_count.is_multiple_of(100) {
            eprintln!("  {} frames sent", frame_count);
        }
    }

    if frame_count == 0 {
        let stderr = source.stderr_output();
        if !stderr.is_empty() {
            eprintln!("ffmpeg stderr:\n{}", stderr.chars().take(2000).collect::<String>());
        }
    }
    println!("Capture ended — {} frames streamed.", frame_count);
    Ok(())
}

fn cmd_lcd_game(system: &System, window: &str, launch: Option<&str>, device: u8, fps: u8) -> Result<()> {
    let lcd_ids = system.lcd_device_ids();
    let device_id = lcd_ids
        .get(device as usize)
        .ok_or_else(|| anyhow::anyhow!("LCD device index {} not found ({} available)", device, lcd_ids.len()))?;

    let lcd_info = system.lcd_device_info();
    let info = lcd_info
        .get(device as usize)
        .ok_or_else(|| anyhow::anyhow!("LCD device info not found for index {}", device))?;
    let (width, height) = (info.width, info.height);

    // Optionally launch the game
    let _child = if let Some(cmd) = launch {
        println!("Launching: {cmd}");
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            anyhow::bail!("empty launch command");
        }
        let child = std::process::Command::new(parts[0])
            .args(&parts[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context(format!("failed to launch '{cmd}'"))?;
        // Wait for the window to appear
        std::thread::sleep(std::time::Duration::from_secs(2));
        Some(child)
    } else {
        None
    };

    println!(
        "Capturing window '{}' → LCD {} ({}x{}, {}fps)",
        window, device, width, height, fps
    );
    println!("Press Ctrl+C to stop");

    let mut source = frgb_lcd::capture::ScreenCaptureSource::new(":0", Some(window), width, height, fps as u32)
        .map_err(|e| anyhow::anyhow!("failed to start screen capture: {e}"))?;

    let lcd = system
        .lcd_ext()
        .ok_or_else(|| anyhow::anyhow!("no LCD backend available"))?;

    let interval = source.frame_interval();
    let mut frame_count = 0u64;

    while let Some(jpeg) = source.next_frame() {
        let start = std::time::Instant::now();
        lcd.send_frame(device_id, &jpeg).context("LCD frame send failed")?;
        frame_count += 1;

        let elapsed = start.elapsed();
        if elapsed < interval {
            std::thread::sleep(interval - elapsed);
        }

        if frame_count.is_multiple_of(300) {
            eprintln!("  {} frames sent", frame_count);
        }
    }

    if frame_count == 0 {
        let stderr = source.stderr_output();
        if !stderr.is_empty() {
            eprintln!("ffmpeg stderr:\n{}", stderr.chars().take(2000).collect::<String>());
        }
    }
    println!("Game capture ended — {} frames streamed.", frame_count);
    Ok(())
}

fn cmd_lcd_h264(system: &System, path: &str, device_idx: u8) -> Result<()> {
    let lcd = system
        .lcd_ext()
        .ok_or_else(|| anyhow::anyhow!("no LCD backend available"))?;

    let devices = lcd.lcd_device_info();
    let dev = devices
        .get(device_idx as usize)
        .ok_or_else(|| anyhow::anyhow!("LCD device {device_idx} not found"))?;

    let device_ids = lcd.lcd_device_ids();
    let device_id = device_ids
        .get(device_idx as usize)
        .ok_or_else(|| anyhow::anyhow!("LCD device {device_idx} not found"))?;

    let data = std::fs::read(path).context("failed to read H.264 file")?;
    let chunks = frgb_lcd::h264::chunk_count(data.len());
    println!(
        "Uploading {} ({} bytes, {} chunks) to {}",
        path,
        data.len(),
        chunks,
        dev.name
    );

    lcd.upload_h264(device_id, &data)?;
    println!("Playback started on {}", dev.name);
    Ok(())
}

// ---------------------------------------------------------------------------
// IPC bridge — convert CLI commands to daemon requests
// ---------------------------------------------------------------------------

/// Convert a CLI command to an IPC Request. Returns None for commands that
/// need interactive/direct access (bind with prompts).
pub fn to_request(command: &Command) -> Option<Request> {
    match command {
        Command::Status { verbose } => Some(if *verbose {
            Request::StatusVerbose
        } else {
            Request::Status
        }),
        Command::Discover { .. } => Some(Request::Discover),
        Command::Speed { percent, group } => {
            if let Some(gid) = group {
                Some(Request::SetSpeed {
                    group: GroupId::new(*gid),
                    mode: SpeedMode::Manual(frgb_model::SpeedPercent::new(*percent)),
                })
            } else {
                Some(Request::SetSpeedAll {
                    target: frgb_model::ipc::Target::All,
                    mode: SpeedMode::Manual(frgb_model::SpeedPercent::new(*percent)),
                })
            }
        }
        Command::Pwm { group } => {
            if let Some(gid) = group {
                Some(Request::SetSpeed {
                    group: GroupId::new(*gid),
                    mode: SpeedMode::Pwm,
                })
            } else {
                Some(Request::SetSpeedAll {
                    target: frgb_model::ipc::Target::All,
                    mode: SpeedMode::Pwm,
                })
            }
        }
        Command::Pump { mode, group } => {
            let pump_mode = parse_pump_mode(mode).ok()?;
            Some(Request::SetPumpMode {
                group: GroupId::new(*group),
                mode: pump_mode,
            })
        }
        Command::RgbOff { group } => {
            if let Some(gid) = group {
                Some(Request::SetRgb {
                    group: GroupId::new(*gid),
                    mode: RgbMode::Off,
                })
            } else {
                Some(Request::SetRgbAll {
                    target: frgb_model::ipc::Target::All,
                    mode: RgbMode::Off,
                })
            }
        }
        Command::MbSync { state, group } => {
            let enable = match state.to_lowercase().as_str() {
                "on" | "1" | "true" => true,
                "off" | "0" | "false" => false,
                _ => return None,
            };
            group.as_ref().map(|gid| Request::SetMbSync {
                group: GroupId::new(*gid),
                enable,
            })
        }
        Command::SetRole { role, group } => parse_role(role).ok().map(|r| Request::SetRole {
            group: GroupId::new(*group),
            role: r,
        }),
        Command::Rename { name, group } => Some(Request::RenameGroup {
            group: GroupId::new(*group),
            name: name.clone(),
        }),
        Command::Lock => Some(Request::Lock),
        Command::Unlock => Some(Request::Unlock),
        Command::Led { .. } => None,    // direct mode only (needs device type for layout)
        Command::Bind => None,          // interactive
        Command::Unbind { .. } => None, // needs rf_ext
        Command::Sensors => Some(Request::ListSensors),
        Command::Play { name, group } => {
            let target = group.map(|g| frgb_model::ipc::Target::Group(GroupId::new(g)));
            Some(Request::StartSequence {
                name: name.clone(),
                target,
            })
        }
        Command::Stop { group } => {
            let target = group.map(|g| frgb_model::ipc::Target::Group(GroupId::new(g)));
            Some(Request::StopSequence { target })
        }
        Command::Color {
            color,
            group,
            ring,
            inner,
            outer,
            inner_top,
            inner_middle,
            inner_bottom,
            outer_top,
            outer_middle,
            outer_bottom,
            brightness,
        } => {
            match build_color_mode(
                color.as_deref(),
                ring.clone(),
                inner.as_deref(),
                outer.as_deref(),
                SubZoneArgs {
                    inner_top: inner_top.as_deref(),
                    inner_middle: inner_middle.as_deref(),
                    inner_bottom: inner_bottom.as_deref(),
                    outer_top: outer_top.as_deref(),
                    outer_middle: outer_middle.as_deref(),
                    outer_bottom: outer_bottom.as_deref(),
                },
                Brightness::new(*brightness),
            ) {
                Ok(mode) => Some(rgb_request(group.map(GroupId::new), mode)),
                Err(_) => None, // fall back to direct mode on parse error
            }
        }
        Command::Effect {
            name,
            color,
            group,
            brightness,
            ring,
            speed,
            direction,
        } => match build_effect_mode(
            name,
            color,
            ring.clone(),
            *speed,
            direction.clone(),
            Brightness::new(*brightness),
        ) {
            Ok(mode) => Some(rgb_request(group.map(GroupId::new), mode)),
            Err(_) => None,
        },
        // Handled in main() before daemon/USB — should not reach here
        Command::ListEffects | Command::ListColors => None,
        // Needs direct USB access for frame streaming
        Command::LcdPlay { .. } | Command::LcdCapture { .. } | Command::LcdGame { .. } | Command::LcdH264 { .. } => {
            None
        }
        // Direct mode only (config file access)
        Command::Mobo { .. } => None,
    }
}

/// Print a daemon Response for the user.
pub fn print_response(command: &Command, response: &Response) -> Result<()> {
    match response {
        Response::Ok => {
            match command {
                Command::Speed { percent, group } => {
                    if let Some(gid) = group {
                        println!("Group {gid}: speed set to {percent}%");
                    } else {
                        println!("All groups: speed set to {percent}%");
                    }
                }
                Command::Pwm { group } => {
                    if let Some(gid) = group {
                        println!("Group {gid}: released to motherboard PWM");
                    } else {
                        println!("All groups: released to motherboard PWM");
                    }
                }
                Command::Pump { mode, group } => println!("Group {group}: pump mode set to {mode}"),
                Command::RgbOff { .. } => println!("RGB off"),
                Command::Play { name, .. } => println!("Sequence '{}' started", name),
                Command::Stop { .. } => println!("Playback stopped"),
                Command::MbSync { state, group } => {
                    let gid_str = group.map_or("All groups".to_string(), |g| format!("Group {g}"));
                    let action = if state.to_lowercase().as_str() == "on" || state == "1" || state == "true" {
                        "motherboard control enabled"
                    } else {
                        "frgb control enabled"
                    };
                    println!("{gid_str}: {action}");
                }
                Command::SetRole { role, group } => println!("Group {group}: role set to {role}"),
                Command::Rename { name, group } => println!("Group {group}: renamed to '{name}'"),
                Command::Lock => println!("All devices: locked"),
                Command::Unlock => println!("All devices: unlocked"),
                _ => {}
            }
            Ok(())
        }
        Response::Error(msg) => anyhow::bail!("{msg}"),
        Response::DeviceStatus(groups) => {
            let verbose = matches!(command, Command::Status { verbose: true });
            if groups.is_empty() {
                println!("No fan groups discovered.");
            } else {
                let mut sorted = groups.clone();
                sorted.sort_by_key(|gs| gs.group.id);

                let mut total_intake_cfm: f32 = 0.0;
                let mut total_exhaust_cfm: f32 = 0.0;

                for gs in &sorted {
                    if matches!(gs.group.device_type, frgb_model::device::DeviceType::Aura) {
                        if !verbose {
                            continue;
                        }
                        let state_str = match gs.rgb {
                            frgb_model::rgb::RgbMode::Off => "off",
                            _ => "managed",
                        };
                        println!("Group {}: {DIM}AURA RGB{RESET} ({state_str})", gs.group.id);
                        continue;
                    }

                    let rpms: Vec<String> = gs.rpms.iter().map(|r| r.to_string()).collect();
                    let rpm_str = if rpms.is_empty() { "n/a".into() } else { rpms.join("/") };

                    let speed_pct: Option<f32> = match &gs.speed {
                        SpeedMode::Manual(pct) => Some(pct.value() as f32),
                        _ => None,
                    };

                    let (speed_str, group_cfm) = if gs.mb_sync {
                        (format!("{YELLOW}MB{RESET}"), None)
                    } else {
                        let s = match &gs.speed {
                            SpeedMode::Manual(pct) => format!("{}%", pct.value()),
                            SpeedMode::Pwm => "PWM".to_string(),
                            SpeedMode::Curve(_) | SpeedMode::NamedCurve(_) => "Curve".to_string(),
                        };
                        let cfm = gs
                            .group
                            .cfm_max
                            .and_then(|cfm_max| speed_pct.map(|pct| cfm_max * (pct / 100.0)));
                        (s, cfm)
                    };

                    let cfm_str = group_cfm.map(|cfm| format!("{cfm:.0} CFM")).unwrap_or_default();

                    if let Some(cfm) = group_cfm {
                        match &gs.group.role {
                            FanRole::Intake => total_intake_cfm += cfm,
                            FanRole::Exhaust => total_exhaust_cfm += cfm,
                            _ => {}
                        }
                    }

                    let (role_color, role_str) = match &gs.group.role {
                        FanRole::Intake => (BLUE, "↓ in"),
                        FanRole::Exhaust => (RED, "↑ out"),
                        FanRole::Pump => (DIM, "⟳ pump"),
                        FanRole::Custom(s) => (DIM, s.as_str()),
                    };

                    println!(
                        "Group {}: {} ({}{}{}) — {} RPM  {}  {}{}{}",
                        gs.group.id,
                        gs.group.name,
                        role_color,
                        role_str,
                        RESET,
                        rpm_str,
                        speed_str,
                        role_color,
                        cfm_str,
                        RESET,
                    );
                }

                if total_intake_cfm > 0.0 || total_exhaust_cfm > 0.0 {
                    println!();
                    println!(
                        "Airflow: {BLUE}↓ {:.0} CFM in{RESET}  {RED}↑ {:.0} CFM out{RESET}  net {:.0} CFM",
                        total_intake_cfm,
                        total_exhaust_cfm,
                        total_intake_cfm - total_exhaust_cfm,
                    );
                    if total_exhaust_cfm > total_intake_cfm {
                        println!("{YELLOW}⚠ Negative pressure — exhaust exceeds intake{RESET}");
                    }
                }
            }
            Ok(())
        }
        Response::SensorList(sensors) => {
            if sensors.is_empty() {
                println!("No sensors detected.");
            } else {
                for s in sensors {
                    println!("{:<20} {:>6.1}°C  ({:?})", s.name, s.current, s.sensor);
                }
            }
            Ok(())
        }
        Response::SensorReading { sensor, value } => {
            println!("{:?}: {:.1}°C", sensor, value);
            Ok(())
        }
        _ => {
            println!("{}", serde_json::to_string_pretty(response).unwrap_or_default());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_color_hex() {
        let c = parse_color("ff0000").unwrap();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn parse_color_hex_with_hash() {
        let c = parse_color("#00ff00").unwrap();
        assert_eq!(c.g, 255);
    }

    #[test]
    fn parse_color_named() {
        let c = parse_color("red").unwrap();
        assert_eq!(c.r, 254);
    }

    #[test]
    fn parse_color_invalid() {
        assert!(parse_color("notacolor").is_err());
    }

    #[test]
    fn parse_color_empty() {
        assert!(parse_color("").is_err());
    }

    #[test]
    fn parse_color_short_hex() {
        assert!(parse_color("f00").is_err());
    }
}

#[cfg(test)]
mod mobo_tests {
    use super::*;
    use frgb_model::config::{Config, HwmonChannelConfig, HwmonChannelRole};

    #[test]
    fn parse_role_accepts_all_variants() {
        assert_eq!(parse_hwmon_role("intake").unwrap(), HwmonChannelRole::Intake);
        assert_eq!(parse_hwmon_role("exhaust").unwrap(), HwmonChannelRole::Exhaust);
        assert_eq!(parse_hwmon_role("pump").unwrap(), HwmonChannelRole::Pump);
        assert_eq!(parse_hwmon_role("fan").unwrap(), HwmonChannelRole::Fan);
        assert_eq!(parse_hwmon_role("INTAKE").unwrap(), HwmonChannelRole::Intake);
    }

    #[test]
    fn parse_role_rejects_bogus() {
        assert!(parse_hwmon_role("turbo").is_err());
    }

    #[test]
    fn upsert_channel_adds_new_entry() {
        let mut cfg = Config::default();
        upsert_hwmon_channel(
            &mut cfg,
            HwmonChannelConfig {
                pwm: 2,
                name: "Rear".into(),
                role: HwmonChannelRole::Exhaust,
                model: None,
                min_pwm: 0,
                curve_execution: Default::default(),
            },
        );
        assert_eq!(cfg.hwmon.channels.len(), 1);
        assert_eq!(cfg.hwmon.channels[0].pwm, 2);
    }

    #[test]
    fn upsert_channel_replaces_existing() {
        let mut cfg = Config::default();
        upsert_hwmon_channel(
            &mut cfg,
            HwmonChannelConfig {
                pwm: 2, name: "Old".into(), role: HwmonChannelRole::Fan,
                model: None, min_pwm: 0, curve_execution: Default::default(),
            },
        );
        upsert_hwmon_channel(
            &mut cfg,
            HwmonChannelConfig {
                pwm: 2, name: "New".into(), role: HwmonChannelRole::Exhaust,
                model: None, min_pwm: 0, curve_execution: Default::default(),
            },
        );
        assert_eq!(cfg.hwmon.channels.len(), 1);
        assert_eq!(cfg.hwmon.channels[0].name, "New");
        assert_eq!(cfg.hwmon.channels[0].role, HwmonChannelRole::Exhaust);
    }

    #[test]
    fn format_hwmon_managed_row_contains_key_fields() {
        let row = format_hwmon_managed_row(
            2,
            "Rear exhaust",
            HwmonChannelRole::Exhaust,
            1700,
            128,
            "manual",
        );
        assert!(row.contains("pwm2"));
        assert!(row.contains("Rear exhaust"));
        assert!(row.contains("1700"));
        assert!(row.contains("50%")); // 128 / 255 ≈ 50%
        assert!(row.contains("manual"));
        assert!(row.contains("exhaust"));
    }

    #[test]
    fn format_hwmon_unnamed_hint_for_single_channel() {
        let hint = format_hwmon_unnamed_hint(&[2]);
        assert!(hint.contains("pwm2"));
        assert!(hint.contains("./r mobo name"));
    }

    #[test]
    fn format_hwmon_unnamed_hint_for_many_channels() {
        let hint = format_hwmon_unnamed_hint(&[2, 3, 5]);
        assert!(hint.contains("pwm2") && hint.contains("pwm3") && hint.contains("pwm5"));
    }

    #[test]
    fn hwmon_mode_label_maps_known_values() {
        assert_eq!(hwmon_mode_label(Some(1)), "manual");
        assert_eq!(hwmon_mode_label(Some(5)), "smart");
        assert_eq!(hwmon_mode_label(Some(3)), "auto");
        assert_eq!(hwmon_mode_label(Some(0)), "off");
        assert_eq!(hwmon_mode_label(Some(9)), "mode:9");
        assert_eq!(hwmon_mode_label(None), "?");
    }
}
