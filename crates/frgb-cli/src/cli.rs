use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "frgb", about = "Linux fan/RGB/LCD controller for Lian Li hardware")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Wireless channel override (default: auto-detect from hardware).
    #[arg(long)]
    pub channel: Option<u8>,

    /// Force direct USB access, bypassing daemon even if running.
    #[arg(long)]
    pub direct: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Show status of all fan groups
    Status {
        #[arg(short, long)]
        verbose: bool,
    },
    /// Discover connected fan groups
    Discover {
        /// Show raw device records from all query passes
        #[arg(long)]
        raw: bool,
    },
    /// Set fan speed percentage (0-100)
    Speed {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
        #[arg(short, long)]
        group: Option<u8>,
    },
    /// Release fans to motherboard PWM control
    Pwm {
        #[arg(short, long)]
        group: Option<u8>,
    },
    /// Set AIO pump speed mode (quiet, standard, high, full, or fixed percentage)
    Pump {
        /// Mode: quiet, standard, high, full, or a number 0-100 for fixed duty
        mode: String,
        /// Group ID of the AIO device
        #[arg(short, long)]
        group: u8,
    },
    /// Set static color. Supports per-ring, per-fan, and TL sub-zone colors.
    ///
    /// Examples:
    ///   frgb color red -g 1                          # all LEDs red
    ///   frgb color red --ring inner -g 1              # inner red, outer off
    ///   frgb color --inner red --outer blue -g 1      # inner red, outer blue
    ///   frgb color red,blue,green -g 1                # fan1=red, fan2=blue, fan3=green
    ///   frgb color --inner-top red --outer-bottom blue -g 1
    ///                                                # TL/SL only: per sub-zone
    Color {
        /// Color for all LEDs (or comma-separated per-fan colors).
        /// Omit when using --inner/--outer or sub-zone flags.
        color: Option<String>,
        #[arg(short, long)]
        group: Option<u8>,
        #[arg(short, long, default_value = "both")]
        ring: RingArg,
        /// Inner ring color (hex or name)
        #[arg(long)]
        inner: Option<String>,
        /// Outer ring color (hex or name)
        #[arg(long)]
        outer: Option<String>,
        /// TL/SL only: inner-top sub-zone color
        #[arg(long)]
        inner_top: Option<String>,
        /// TL/SL only: inner-middle sub-zone color
        #[arg(long)]
        inner_middle: Option<String>,
        /// TL/SL only: inner-bottom sub-zone color
        #[arg(long)]
        inner_bottom: Option<String>,
        /// TL/SL only: outer-top sub-zone color
        #[arg(long)]
        outer_top: Option<String>,
        /// TL/SL only: outer-middle sub-zone color
        #[arg(long)]
        outer_middle: Option<String>,
        /// TL/SL only: outer-bottom sub-zone color
        #[arg(long)]
        outer_bottom: Option<String>,
        /// Brightness 0-255
        #[arg(short, long, default_value = "255")]
        brightness: u8,
    },
    /// Turn off RGB LEDs
    #[command(name = "rgb-off")]
    RgbOff {
        #[arg(short, long)]
        group: Option<u8>,
    },
    /// Apply an RGB effect (use list-effects to see all)
    Effect {
        /// Effect name: breathing, meteor, runway
        name: String,
        /// Color as hex (ff0000) or name (red, blue, green)
        #[arg(short, long, default_value = "red")]
        color: String,
        #[arg(short, long)]
        group: Option<u8>,
        /// Brightness 0-255
        #[arg(short, long, default_value = "255")]
        brightness: u8,
        /// Ring selection: both, inner, outer
        #[arg(short, long, default_value = "both")]
        ring: RingArg,
        /// Speed 1-5 (1=slowest, 5=fastest)
        #[arg(short, long, default_value = "3")]
        speed: u8,
        /// Direction: cw, ccw
        #[arg(short, long, default_value = "cw")]
        direction: DirectionArg,
    },
    /// Set a single LED per fan to a color (rest off). Per-fan colors via comma separation.
    ///
    /// Examples:
    ///   frgb led red -g 1                  # LED 0 red on all fans
    ///   frgb led red,green,blue -g 1       # fan1=red, fan2=green, fan3=blue
    ///   frgb led cyan -g 5 --index 3       # LED 3 cyan on all fans
    Led {
        /// Color(s), comma-separated for per-fan
        color: String,
        #[arg(short, long)]
        group: u8,
        /// Which LED index to light (default: 0)
        #[arg(short, long, default_value = "0")]
        index: usize,
    },
    /// List available temperature/fan sensors
    Sensors,
    /// Start a named sequence
    #[command(name = "play")]
    Play {
        /// Sequence name from config
        name: String,
        #[arg(short, long)]
        group: Option<u8>,
    },
    /// Stop active sequence/effect cycle
    Stop {
        #[arg(short, long)]
        group: Option<u8>,
    },
    /// Set the role for a fan group (intake, exhaust, pump, or custom)
    #[command(name = "set-role")]
    SetRole {
        /// Role: intake, exhaust, pump, or a custom string
        role: String,
        #[arg(short, long)]
        group: u8,
    },
    /// Rename a fan group
    Rename {
        /// New display name
        name: String,
        #[arg(short, long)]
        group: u8,
    },
    /// Toggle motherboard PWM sync for a group
    ///
    /// Devices in MB sync ignore speed commands from frgb.
    /// Use `mbsync off` to take control, `mbsync on` to hand back to motherboard.
    #[command(name = "mbsync")]
    MbSync {
        /// on or off
        state: String,
        #[arg(short, long)]
        group: Option<u8>,
    },
    /// Bind an unbound device to a group (interactive)
    Bind,
    /// Unbind a device from its group
    Unbind {
        #[arg(short, long)]
        group: u8,
    },
    /// Lock all devices to this controller
    Lock,
    /// Unlock all devices
    Unlock,
    /// List all available RGB effects
    #[command(name = "list-effects")]
    ListEffects,
    /// List named colors
    #[command(name = "list-colors")]
    ListColors,
    /// Stream screen capture to LCD
    #[command(name = "lcd-capture")]
    LcdCapture {
        /// X11 display (default: :0)
        #[arg(long, default_value = ":0")]
        display: String,
        /// Window title to capture (default: full screen)
        #[arg(short, long)]
        window: Option<String>,
        /// LCD device index
        #[arg(short, long, default_value = "0")]
        device: u8,
        /// Target FPS
        #[arg(long, default_value = "30")]
        fps: u8,
    },
    /// Stream a game window to LCD
    #[command(name = "lcd-game")]
    LcdGame {
        /// Window title to capture (e.g., "Doom", "Chocolate Doom")
        #[arg(short, long)]
        window: String,
        /// Optional: command to launch the game first
        #[arg(short, long)]
        launch: Option<String>,
        /// LCD device index
        #[arg(short, long, default_value = "0")]
        device: u8,
        /// Target FPS
        #[arg(long, default_value = "30")]
        fps: u8,
    },
    /// Stream a video, GIF, or image file to an LCD screen via ffmpeg
    #[command(name = "lcd-play")]
    LcdPlay {
        /// Path to video/image/GIF file
        path: String,
        /// LCD device index (0 = first)
        #[arg(short, long, default_value = "0")]
        device: u8,
        /// Target FPS (default: 24)
        #[arg(long, default_value = "24")]
        fps: u8,
    },
    /// Upload and play H.264 video on LCD (on-device decoder)
    #[command(name = "lcd-h264")]
    LcdH264 {
        /// Path to H.264 file
        path: String,
        /// LCD device index
        #[arg(short, long, default_value = "0")]
        device: u8,
    },
    /// Manage motherboard-header fans (hwmon backend).
    Mobo {
        #[command(subcommand)]
        action: MoboAction,
    },
}

#[derive(Subcommand)]
pub enum MoboAction {
    /// Name a detected pwm channel so it becomes manageable.
    Name {
        /// pwm channel index (1-based), e.g. 2 for pwm2.
        pwm: u8,
        /// Display name for this channel, e.g. "Rear exhaust".
        name: String,
        /// Fan role.
        #[arg(long, default_value = "fan")]
        role: String,
        /// Optional fan model for CFM lookup.
        #[arg(long)]
        model: Option<String>,
        /// Safety floor as a percentage (0-100); mapped to 0-255 PWM byte.
        #[arg(long)]
        min: Option<u8>,
    },
}

#[derive(Clone, ValueEnum)]
pub enum RingArg {
    Both,
    Inner,
    Outer,
}

impl From<RingArg> for frgb_model::rgb::Ring {
    fn from(r: RingArg) -> Self {
        match r {
            RingArg::Both => Self::Both,
            RingArg::Inner => Self::Inner,
            RingArg::Outer => Self::Outer,
        }
    }
}

#[derive(Clone, ValueEnum)]
pub enum DirectionArg {
    Cw,
    Ccw,
}

impl From<DirectionArg> for frgb_model::rgb::EffectDirection {
    fn from(d: DirectionArg) -> Self {
        match d {
            DirectionArg::Cw => Self::Cw,
            DirectionArg::Ccw => Self::Ccw,
        }
    }
}
