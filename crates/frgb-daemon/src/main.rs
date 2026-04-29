mod alerts;
mod app_profiles;
mod config_cache;
mod curves;
mod engine;
mod handler;
mod ipc;
mod lcd_manager;
mod openrgb_server;
mod power;
mod scheduler;
mod show_runner;
mod temp_rgb;

use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use frgb_core::{AuraBackend, HwmonBackend, LcdBackend, LianLiRfBackend, OpenRgbBackend, System, UsbTransport, WiredEneBackend};
use frgb_model::spec_loader;
use frgb_model::usb_ids::{PID_RX, PID_TX, VID_LIANLI};

/// Global shutdown flag — set by signal handler, checked each tick.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Effective hwmon state-file path, set after `HwmonBackend::open` succeeds so
/// the panic hook targets the same file the backend writes to (honors any
/// `hwmon.state_file` config override). Unset before backend init completes.
static HWMON_STATE_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

fn main() -> Result<()> {
    // Log to both stderr and a file so crashes are diagnosable even when
    // the daemon is auto-started by the GUI (which drains stderr silently).
    let log_path = dirs::runtime_dir()
        .or_else(|| std::env::var_os("TMPDIR").map(std::path::PathBuf::from))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("frgbd.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();

    use tracing_subscriber::prelude::*;
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_timer(tracing_subscriber::fmt::time::uptime());
    let file_layer = log_file.map(|f| {
        tracing_subscriber::fmt::layer()
            .with_timer(tracing_subscriber::fmt::time::uptime())
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(f))
    });
    tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .with(file_layer)
        .init();

    // Panic hook: write crash info to log file before aborting
    {
        let crash_path = log_path.clone();
        std::panic::set_hook(Box::new(move |info| {
            let msg = format!("DAEMON PANIC: {info}\n");
            eprintln!("{msg}");
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&crash_path) {
                use std::io::Write;
                let _ = f.write_all(msg.as_bytes());
            }
            // Best-effort hwmon restore. Prefer the path the backend actually
            // wrote to (honors config override); fall back to default if the
            // backend hasn't initialized yet.
            let state_path = HWMON_STATE_PATH
                .get()
                .cloned()
                .unwrap_or_else(frgb_core::hwmon_backend::state::default_state_path);
            frgb_core::hwmon_backend::emergency_restore(
                std::path::Path::new("/sys/class/hwmon"),
                &state_path,
            );
        }));
    }

    tracing::info!("frgbd log: {}", log_path.display());

    // Register signal handlers for graceful shutdown
    unsafe {
        let handler: extern "C" fn(libc::c_int) = signal_handler;
        libc::signal(libc::SIGTERM, handler as libc::sighandler_t);
        libc::signal(libc::SIGINT, handler as libc::sighandler_t);
    }

    let channel_override: Option<u8> = std::env::args()
        .position(|a| a == "--channel")
        .and_then(|i| std::env::args().nth(i + 1))
        .and_then(|s| s.parse().ok());

    tracing::info!("frgbd starting");

    // Bind IPC socket early so the GUI can connect while backends initialize
    let sock_path = ipc::socket_path();
    let server = ipc::IpcServer::bind(&sock_path).context(format!(
        "Failed to bind IPC socket at {}. Is another frgbd already running?",
        sock_path.display()
    ))?;
    tracing::info!("IPC ready at {}", sock_path.display());

    let specs = spec_loader::load_with_overrides();
    let mut system = System::new(specs);

    // Load config into in-memory cache early (needed for backend init)
    let mut config_cache = config_cache::ConfigCache::load();

    // Open Lian Li RF wireless backends (supports multiple TX/RX pairs)
    {
        let tx_devices = frgb_usb::device::UsbDevice::open_all(VID_LIANLI, PID_TX);
        let rx_devices = frgb_usb::device::UsbDevice::open_all(VID_LIANLI, PID_RX);
        let pair_count = tx_devices.len().min(rx_devices.len());
        if pair_count == 0 {
            // Fall back to single open for better error messages
            match UsbTransport::open(VID_LIANLI, PID_TX)
                .and_then(|tx| UsbTransport::open(VID_LIANLI, PID_RX).map(|rx| (tx, rx)))
            {
                Ok((tx, rx)) => {
                    let backend = LianLiRfBackend::new(tx, rx, channel_override);
                    system.add_backend(Box::new(backend));
                    tracing::info!("RF: wireless controller found");
                }
                Err(e) => tracing::warn!("RF wireless controller not available: {e}"),
            }
        } else {
            let mut tx_iter = tx_devices.into_iter();
            let mut rx_iter = rx_devices.into_iter();
            for i in 0..pair_count {
                let tx = UsbTransport::from_device(tx_iter.next().unwrap());
                let rx = UsbTransport::from_device(rx_iter.next().unwrap());
                let id = frgb_core::BackendId(i as u8);
                let backend = LianLiRfBackend::with_id(id, tx, rx, channel_override);
                system.add_backend(Box::new(backend));
            }
            tracing::info!("RF: {} wireless controller pair(s) found", pair_count);
        }
    }

    // Open AURA motherboard RGB (if present)
    match AuraBackend::open_all(&config_cache.config().aura) {
        Ok(aura) if aura.channel_count() > 0 => {
            tracing::info!("AURA: {} channel(s) found", aura.channel_count());
            system.add_backend(Box::new(aura));
        }
        Ok(_) => tracing::debug!("No AURA devices found"),
        Err(e) => tracing::warn!("AURA init failed: {e}"),
    }

    // Open wired ENE fan hubs (if present)
    match WiredEneBackend::open_all() {
        Ok(ene) if ene.device_count() > 0 => {
            tracing::info!("ENE: {} device(s) found", ene.device_count());
            system.add_backend(Box::new(ene));
        }
        Ok(_) => tracing::debug!("No wired ENE devices found"),
        Err(e) => tracing::warn!("ENE init failed: {e}"),
    }

    // Open LCD devices (if present)
    match LcdBackend::open_all() {
        Ok(lcd) if lcd.device_count() > 0 => {
            tracing::info!("LCD: {} device(s) found", lcd.device_count());
            system.add_backend(Box::new(lcd));
        }
        Ok(_) => tracing::debug!("No LCD devices found"),
        Err(e) => tracing::warn!("LCD init failed: {e}"),
    }

    // Connect to OpenRGB server (if running)
    match OpenRgbBackend::open_default() {
        Ok(orgb) => {
            tracing::info!("OpenRGB: connected to server");
            system.add_backend(Box::new(orgb));
        }
        Err(e) => tracing::debug!("OpenRGB not available: {e}"),
    }

    // Open hwmon motherboard fan backend (non-fatal if no supported chip)
    match HwmonBackend::open(&config_cache.config().hwmon) {
        Ok(hwmon) if hwmon.channel_count() > 0 || !hwmon.unconfigured_channels().is_empty() => {
            tracing::info!(
                "hwmon: {} configured channel(s), {} unconfigured on '{}'",
                hwmon.channel_count(),
                hwmon.unconfigured_channels().len(),
                hwmon.chip_name()
            );
            let _ = HWMON_STATE_PATH.set(hwmon.state_path().to_path_buf());
            system.add_backend(Box::new(hwmon));
        }
        Ok(_) => tracing::debug!("No hwmon chip found"),
        Err(e) => tracing::warn!("hwmon init failed: {e}"),
    }

    // Initial discovery (non-fatal — daemon runs with 0 devices if needed)
    match system.discover() {
        Ok(()) => tracing::info!("Discovered {} device group(s)", system.devices().len()),
        Err(e) => tracing::warn!("Discovery error (will retry): {e}"),
    }

    // Warn on any group-id collisions across backends (spec §4.3 for hwmon;
    // good hygiene for all backends).
    system.warn_group_id_overlaps();

    // Config already loaded above; just read poll interval
    let poll_interval = config_cache.config().daemon.poll_interval_ms;

    // Apply user-configured group properties (role, name)
    system.registry.apply_group_configs(&config_cache.config().groups);
    system.registry.seed_state_from_config(&config_cache.config().groups);

    // Engine for periodic tasks (includes show runner + curve runner)
    let mut engine = engine::Engine::new(poll_interval);

    // Load fan curves, sequences, alerts, and hwmon sensors from config
    engine.curves.sync_from_config(&system, config_cache.config());
    engine.show_runner.load_sequences(config_cache.config());
    if let Some(alert_config) = config_cache.config().alerts.clone() {
        engine.alerts.set_config(alert_config);
    }
    engine.temp_rgb.sync_from_config(config_cache.config());
    engine.scheduler.load(config_cache.config().schedules.clone());
    engine.app_profiles.load(config_cache.config().app_profiles.clone());
    if let Some(power_config) = config_cache.config().power.clone() {
        engine.power.set_config(power_config);
    }
    engine.init_hwmon(config_cache.config().sensor_calibration.clone());
    engine.load_wear_stats(&config_cache.config().wear_stats);

    // Start OpenRGB SDK server if enabled
    let openrgb = if config_cache.config().daemon.openrgb_server_enabled {
        let caps = build_openrgb_caps(&system);
        let stop = Arc::new(AtomicBool::new(false));
        Some((
            openrgb_server::OpenRgbServer::start(config_cache.config().daemon.openrgb_server_port, caps, stop.clone()),
            stop,
        ))
    } else {
        None
    };

    // Active client connections
    let mut clients: Vec<ipc::IpcConnection> = Vec::new();

    tracing::info!("frgbd ready (poll={}ms, socket={})", poll_interval, sock_path.display());

    // Main loop — runs until SIGTERM/SIGINT
    let tick_interval = Duration::from_millis(50);
    while !SHUTDOWN.load(Ordering::Relaxed) {
        let tick_start = Instant::now();

        // Accept new IPC connections
        while let Some(client) = server.accept() {
            tracing::debug!("IPC client connected");
            clients.push(client);
        }

        // Process IPC requests, removing clients that fail to respond
        let mut request_events = Vec::new();
        clients.retain_mut(|client| {
            match client.read_request() {
                Ok(Some(request)) => {
                    tracing::debug!("IPC request: {:?}", std::mem::discriminant(&request));
                    if matches!(
                        &request,
                        frgb_model::ipc::Request::Subscribe { .. } | frgb_model::ipc::Request::Watch { .. }
                    ) {
                        client.subscribed = true;
                    }
                    let (response, events) = handler::handle(&mut system, &mut engine, &mut config_cache, &request);
                    request_events.extend(events);
                    if client.send_response(&response).is_err() {
                        tracing::debug!("IPC client disconnected (send failed)");
                        return false;
                    }
                }
                Ok(None) => {
                    // No data — keep client
                }
                Err(e) => {
                    tracing::warn!("IPC client protocol error: {e}");
                    let _ = client.send_response(&frgb_model::ipc::Response::Error(format!("protocol error: {e}")));
                    return false; // drop client
                }
            }
            true
        });

        // Engine tick — sequences, discovery, curves
        let mut events = engine.tick(&mut system, &config_cache);
        events.extend(request_events);

        // Dispatch events to subscribed clients only
        if !events.is_empty() {
            clients.retain_mut(|client| {
                if !client.subscribed {
                    return true; // keep non-subscribed clients, just don't send events
                }
                for event in &events {
                    if client.send_event(event).is_err() {
                        tracing::debug!("IPC client disconnected (event send failed)");
                        return false;
                    }
                }
                true
            });
        }

        // Apply OpenRGB colour commands
        if let Some((ref openrgb_srv, _)) = openrgb {
            for cmd in openrgb_srv.drain_commands() {
                match cmd {
                    openrgb_server::OpenRgbCommand::SetLeds { group_id, colors } => {
                        let rgb_colors: Vec<frgb_model::rgb::Rgb> = colors
                            .iter()
                            .map(|c| frgb_model::rgb::Rgb {
                                r: c[0],
                                g: c[1],
                                b: c[2],
                            })
                            .collect();
                        let layout = frgb_rgb::layout::LedLayout::for_device(
                            system
                                .registry
                                .find_by_group(group_id)
                                .map(|d| d.device_type)
                                .unwrap_or(frgb_model::device::DeviceType::SlWireless),
                        );
                        let fan_count = system
                            .registry
                            .find_by_group(group_id)
                            .map(|d| d.fan_count())
                            .unwrap_or(1) as usize;
                        let per_fan = layout.total_per_fan as usize;
                        let mut assignments = Vec::with_capacity(fan_count);
                        for f in 0..fan_count {
                            let start = f * per_fan;
                            let end = (start + per_fan).min(rgb_colors.len());
                            let fan_colors = if start < rgb_colors.len() {
                                &rgb_colors[start..end]
                            } else {
                                &[]
                            };
                            let inner_n = layout.inner_count as usize;
                            let outer_n = layout.outer_count as usize;
                            let inner = fan_colors.get(..inner_n).unwrap_or(fan_colors).to_vec();
                            let outer = fan_colors.get(inner_n..inner_n + outer_n).unwrap_or(&[]).to_vec();
                            assignments.push(frgb_model::rgb::FanLedAssignment { inner, outer });
                        }
                        let mode = frgb_model::rgb::RgbMode::PerLed(assignments);
                        if let Err(e) = system.set_rgb(group_id, &mode) {
                            tracing::debug!("OpenRGB SetLeds group {group_id}: {e}");
                        }
                    }
                    openrgb_server::OpenRgbCommand::SetZoneLeds {
                        group_id,
                        zone_idx,
                        colors,
                    } => {
                        tracing::debug!(
                            "OpenRGB SetZoneLeds group {group_id} zone {zone_idx}: {} colors",
                            colors.len()
                        );
                        // TODO: apply per-zone (single fan) update
                    }
                }
            }
        }

        // Sleep remainder of tick interval
        let elapsed = tick_start.elapsed();
        if elapsed < tick_interval {
            std::thread::sleep(tick_interval - elapsed);
        }
    }

    // Shut down OpenRGB server and join its thread
    if let Some((openrgb_srv, stop)) = openrgb {
        openrgb_srv.shutdown(&stop);
    }

    config_cache.config_mut().wear_stats = engine.wear_entries();
    config_cache.flush();

    tracing::info!("frgbd shutting down");

    // Shut down AURA backend: set Managed channels to Off
    if let Some(aura) = system
        .backend_by_name("aura")
        .and_then(|b| b.as_any().downcast_ref::<frgb_core::AuraBackend>())
    {
        aura.shutdown();
    }

    // Shut down hwmon backend: restore saved pwm_enable values.
    if let Some(hwmon) = system
        .backend_by_name("hwmon")
        .and_then(|b| b.as_any().downcast_ref::<frgb_core::HwmonBackend>())
    {
        hwmon.shutdown();
    }

    // IpcServer::drop() removes the socket file
    drop(server);
    Ok(())
}

extern "C" fn signal_handler(_sig: libc::c_int) {
    SHUTDOWN.store(true, Ordering::Relaxed);
}

fn build_openrgb_caps(system: &System) -> Vec<openrgb_server::DeviceCapabilities> {
    use frgb_rgb::layout::LedLayout;

    system
        .devices()
        .iter()
        .map(|dev| {
            let layout = LedLayout::for_device(dev.device_type);
            let fan_count = dev.fan_count();
            let total = layout.total_leds(fan_count) as u16;
            let zones = (0..fan_count)
                .map(|i| openrgb_server::ZoneInfo {
                    name: format!("Fan {}", i + 1),
                    led_count: layout.total_per_fan as u16,
                })
                .collect();
            openrgb_server::DeviceCapabilities {
                device_id: format!("{}", dev.id),
                device_name: dev.name.clone(),
                group_id: dev.group,
                zones,
                total_leds: total,
            }
        })
        .collect()
}
