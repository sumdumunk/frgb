# frgb

Linux fan, RGB, and LCD controller for Lian Li hardware. A clean-room reimplementation of L-Connect 3.

## Supported Hardware

| Device | Type | Features |
|--------|------|----------|
| UNI FAN SL V2 / SL-INF | Wireless (RF) | RGB (58 effects), speed, per-LED color |
| UNI FAN AL | Wireless (RF) | RGB, speed |
| UNI FAN TL | Wireless (RF) | RGB (28 TL-specific modes), speed |
| UNI FAN CL | Wireless (RF) | RGB, speed, MB sync |
| RL120 | Wireless (RF) | RGB, speed |
| HydroShift II (Circle/Square) | Wireless (RF) + LCD USB | RGB, pump speed, 480x480 LCD |
| SL-LCD Wireless | LCD USB | 400x400 LCD |
| TL V2 LCD | LCD USB | 400x400 LCD |
| UNI HUB (ENE 6K77) | Wired USB | RGB, speed (7 model variants) |
| AURA motherboard headers | HID | Addressable RGB (per-channel) |
| OpenRGB devices | Network | RGB via OpenRGB protocol |

## Architecture

```
frgb-model      Data types, config, IPC protocol, device specs
frgb-protocol   RF wire encoding, pump RPM-to-PWM scaling
frgb-usb        USB transport (hidraw bulk I/O)
frgb-rgb        58 effect generators, TUZ compression, per-LED composition
frgb-lcd        JPEG/H.264 encoding, encryption, video/screen capture streaming
frgb-lcd-render Template widget engine (gauges, bars, text, images)
frgb-ipc        Shared IPC message types
frgb-core       Backend abstraction (RF, wired, AURA, LCD, OpenRGB), device registry
frgb-daemon     IPC server, engine loop, fan curves, alerts, schedules, OpenRGB SDK server
frgb-cli        CLI with daemon IPC + direct USB fallback
frgb-gui        Slint-based GUI (sidebar, RGB/LED/speed/LCD editors)
```

## Building

```bash
# Debug build
cargo build --workspace

# Release build
cargo build --release

# Run tests
cargo test --workspace
```

Requires: Rust 2021 edition, `pkg-config`, `libudev-dev`, `libhidapi-dev`.

For LCD features: `ffmpeg` (video/capture streaming), `xdotool` (window capture).

### Linting

```bash
./c                                                    # clippy with -D warnings
cargo clippy --workspace --all-targets -- -D warnings  # equivalent
```

All PRs must pass this check; the codebase is clippy-clean under denied warnings.

## Usage

### Daemon + CLI

```bash
# Start the daemon (needs USB access — run as root or configure udev rules)
sudo target/release/frgb-daemon

# In another terminal, use the CLI (connects to daemon via IPC)
frgb status
frgb speed 60
frgb speed 60 -g 2          # group 2 only
frgb pwm                     # release to motherboard PWM
frgb color red
frgb color --inner red --outer blue
frgb effect rainbow
frgb effect breathing --color cyan --speed 3
frgb pump quiet              # AIO pump: quiet/standard/high/full
frgb mbsync on -g 3          # enable motherboard sync for group 3
frgb sensors                 # list temperature/fan sensors
```

### Direct USB mode (no daemon)

```bash
frgb --direct status
frgb --direct speed 80
frgb --direct bind           # interactive device binding
frgb --direct lock
```

### LCD commands

```bash
# Stream a video file to LCD
frgb lcd-play video.mp4 --fps 24

# Capture screen to LCD
frgb lcd-capture --fps 30
frgb lcd-capture --window "Firefox" --fps 30

# Stream a game window to LCD
frgb lcd-game --window "Chocolate Doom" --launch "chocolate-doom" --fps 30

# Upload H.264 for on-device playback (no streaming overhead)
frgb lcd-h264 video.h264
```

### Profiles and scheduling

```bash
# Via daemon IPC (GUI or CLI)
frgb play "night-mode"       # start a named sequence
frgb stop                    # stop playback
```

### systemd service

```bash
# Install
cp systemd/frgb-daemon.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now frgb-daemon

# Check status
systemctl --user status frgb-daemon
journalctl --user -u frgb-daemon -f
```

## GUI

The Slint-based GUI connects to the daemon via IPC. Tabs for each device group:

- **Overview** — device list with RPM, speed, temperature
- **Speed** — manual %, PWM, fan curves with drag-to-edit points
- **RGB** — effect selector, color picker, per-ring inner/outer control
- **LED** — per-fan zone editor and per-LED click-to-color editor with preset save/load
- **LCD** — content type selector, sensor overlays, brightness/rotation
- **Settings** — profiles, schedules, sync config with role/group filters, calibration

```bash
target/release/frgb-gui
```

## OpenRGB Integration

frgb exposes devices to OpenRGB clients (SignalRGB, etc.) via the OpenRGB SDK protocol:

```json
// In config (~/.config/frgb/config.json), enable:
{
  "daemon": {
    "openrgb_server_enabled": true,
    "openrgb_server_port": 6743
  }
}
```

Supports protocol v0-4, per-LED direct color mode.

## LCD Template System

Composable widget-based LCD display with sensor binding:

```json
{
  "id": "my-dashboard",
  "name": "CPU Dashboard",
  "base_width": 480,
  "base_height": 480,
  "background": { "Color": { "rgba": [10, 10, 20, 255] } },
  "widgets": [
    {
      "id": "cpu-gauge",
      "kind": {
        "RadialGauge": {
          "source": "CpuTemp",
          "value_min": 20.0, "value_max": 100.0,
          "start_angle": 135.0, "sweep_angle": 270.0,
          "inner_radius_pct": 0.78,
          "background_color": [40, 40, 40, 255],
          "ranges": [
            { "max": 60.0, "color": [0, 200, 100], "alpha": 255 },
            { "max": 80.0, "color": [255, 200, 0], "alpha": 255 },
            { "max": null, "color": [255, 50, 50], "alpha": 255 }
          ]
        }
      },
      "x": 240.0, "y": 240.0,
      "width": 350.0, "height": 350.0,
      "visible": true,
      "update_interval_ms": 1000
    }
  ]
}
```

Widget types: Label, ValueText, RadialGauge, VerticalBar, HorizontalBar, Speedometer, CoreBars, Image.

Sensor sources: CpuTemp, GpuTemp, GpuUsage, WaterTemp, CpuUsage, MemUsage, Hwmon (custom), Constant.

Templates use dirty-flag optimization — frames are only re-rendered when sensor values change beyond 0.1 resolution.

## RF Protocol

Communicates with Lian Li wireless devices via a USB TX/RX dongle pair. The protocol uses 240-byte RF frames split into 4x64-byte USB packets, with per-device MAC addressing and group assignment.

- Auto-detects RF channel from bound devices
- Falls back to scanning all 15 channels if no devices found
- Supports bind/unbind/lock/unlock operations
- AIO pump control via RF aio_param frames (HydroShift II)

## License

MIT
