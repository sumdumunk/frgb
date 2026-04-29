//! USB Vendor and Product IDs for all supported Lian Li and related hardware.
//! This is the single source of truth — all other crates import from here.

// Lian Li main controller (TX/RX wireless fan hub)
pub const VID_LIANLI: u16 = 0x0416;
pub const PID_TX: u16 = 0x8040;
pub const PID_RX: u16 = 0x8041;

// LCD panel variants (vendor: Luminary Micro / TI)
// Source: L-Connect.Core ControllerTypeID enum
pub const VID_LCD: u16 = 0x1CBE;
pub const PID_SL_LCD: u16 = 0x0005; // SLV3 LCD (400x400)
pub const PID_TLV2_LCD: u16 = 0x0006; // TLV2 LCD (400x400)
pub const PID_HYDROSHIFT_CIRCLE: u16 = 0xA021; // HydroShift II Circle (480x480)
pub const PID_HYDROSHIFT_SQUARE: u16 = 0xA034; // HydroShift II Square (480x480)
pub const PID_UNIVERSAL_88: u16 = 0xA088; // Universal 8.8" screen (480x1920)

// UNI HUB (wired fan hub, CH340 USB-serial chip)
pub const VID_HUB: u16 = 0x1A86;
pub const PID_HUB: u16 = 0x8091;
pub const PID_SLV3H: u16 = 0x2107;

// Strimer wireless controller
pub const VID_STRIMER: u16 = 0x0CF2;
pub const PID_STRIMER: u16 = 0xA200;

// ENE 6K77 wired fan controllers (SL/AL/SL-INF/V2 variants)
// VID shared with Strimer (0x0CF2), distinguished by PID range 0xA100-0xA106.
// V2 variants (A103/A104/A106) support 6 fan groups; V1 (A100/A101/A102) support 4.
pub const VID_ENE: u16 = 0x0CF2;
pub const PID_ENE_SL: u16 = 0xA100;
pub const PID_ENE_AL: u16 = 0xA101;
pub const PID_ENE_SL_INF: u16 = 0xA102;
pub const PID_ENE_AL_V2: u16 = 0xA103;
pub const PID_ENE_SL_V2: u16 = 0xA104;
pub const PID_ENE_UNKNOWN: u16 = 0xA105;
pub const PID_ENE_SL_INF2: u16 = 0xA106;

// ASUS AURA (motherboard RGB)
pub const VID_AURA: u16 = 0x0B05;
pub const PID_AURA: u16 = 0x1AA6;
