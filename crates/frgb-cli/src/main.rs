mod cli;
mod commands;

use anyhow::{Context, Result};
use clap::Parser;

use frgb_core::{AuraBackend, HwmonBackend, LianLiRfBackend, System, UsbTransport, WiredEneBackend};
use frgb_ipc::{self as ipc_client, IpcClient, PROTOCOL_VERSION};
use frgb_model::spec_loader;
use frgb_model::usb_ids::{PID_RX, PID_TX, VID_LIANLI};

fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    // Pure lookup commands — no USB or daemon needed
    match &cli.command {
        cli::Command::ListEffects => return commands::cmd_list_effects(),
        cli::Command::ListColors => return commands::cmd_list_colors(),
        _ => {}
    }

    // Try daemon first (if running), fall back to direct USB
    if !cli.direct {
        if let Some(response) = try_daemon(&cli) {
            return response;
        }
    }

    // Direct USB mode — every backend is optional so the CLI stays usable
    // when only some hardware is present (e.g. AURA-only setup with no
    // Lian Li receiver, or Lian Li without AURA).
    let specs = spec_loader::load_with_overrides();
    let mut system = System::new(specs);

    // Config is loaded early because AURA backend needs its channel config.
    let config = frgb_core::config::load_config().unwrap_or_else(|e| {
        eprintln!("Warning: config load failed: {e}");
        frgb_model::config::Config::default()
    });

    // Lian Li RF wireless (non-fatal if not present)
    match UsbTransport::open(VID_LIANLI, PID_TX)
        .and_then(|tx| UsbTransport::open(VID_LIANLI, PID_RX).map(|rx| (tx, rx)))
    {
        Ok((tx, rx)) => {
            let backend = LianLiRfBackend::new(tx, rx, cli.channel);
            system.add_backend(Box::new(backend));
        }
        Err(e) => eprintln!("Warning: Lian Li RF controller not available: {e}"),
    }

    // AURA motherboard RGB (non-fatal if not present)
    match AuraBackend::open_all(&config.aura) {
        Ok(aura) if aura.channel_count() > 0 => {
            system.add_backend(Box::new(aura));
        }
        Ok(_) => {}
        Err(e) => eprintln!("Warning: AURA init failed: {e}"),
    }

    // Wired ENE fan hubs (non-fatal if not present)
    if let Ok(ene) = WiredEneBackend::open_all() {
        if ene.device_count() > 0 {
            system.add_backend(Box::new(ene));
        }
    }

    // LCD devices (non-fatal if not present)
    if let Ok(lcd) = frgb_core::LcdBackend::open_all() {
        if lcd.device_count() > 0 {
            system.add_backend(Box::new(lcd));
        }
    }

    // Hwmon motherboard fans (non-fatal if no supported chip)
    match HwmonBackend::open(&config.hwmon) {
        Ok(hwmon) if hwmon.channel_count() > 0 || !hwmon.unconfigured_channels().is_empty() => {
            system.add_backend(Box::new(hwmon));
        }
        Ok(_) => {}
        Err(e) => eprintln!("Warning: hwmon init failed: {e}"),
    }

    // Refuse to run with zero backends — no hardware means nothing to do.
    if system.backend_count() == 0 {
        anyhow::bail!("No hardware backends available (no Lian Li receiver, no AURA, no wired ENE hub, no LCD). Check USB connections and permissions.");
    }

    system.discover().context("Device discovery failed")?;

    // Warn on any group-id collisions across backends (spec §4.3 for hwmon;
    // good hygiene for all backends).
    system.warn_group_id_overlaps();

    // Apply user-configured group properties (role, name) from config
    system.registry.apply_group_configs(&config.groups);

    commands::dispatch(&mut system, &cli.command)
}

/// Try to send the command via daemon IPC. Returns None if daemon isn't running.
fn try_daemon(cli: &cli::Cli) -> Option<Result<()>> {
    let path = ipc_client::socket_path();
    let mut client = IpcClient::connect(&path).ok()?;

    // Version handshake — warn on mismatch but continue
    match client.call(&frgb_model::ipc::Request::Hello {
        protocol_version: PROTOCOL_VERSION,
    }) {
        Ok(frgb_model::ipc::Response::Hello { .. }) => {}
        Ok(frgb_model::ipc::Response::Error(msg)) => {
            eprintln!("Warning: {msg}");
        }
        Ok(_) | Err(_) => {}
    }

    // Convert CLI command to IPC Request (None = command needs direct access)
    let request = commands::to_request(&cli.command)?;

    let response = match client.call(&request) {
        Ok(resp) => resp,
        Err(e) => return Some(Err(anyhow::anyhow!("daemon IPC error: {e}"))),
    };

    Some(commands::print_response(&cli.command, &response))
}
