pub const PACKET_SIZE: usize = 64;

// USB device IDs — canonical definitions live in frgb_model::usb_ids
pub use frgb_model::usb_ids::{
    PID_AURA, PID_HUB, PID_HYDROSHIFT_CIRCLE, PID_HYDROSHIFT_SQUARE, PID_RX, PID_SLV3H, PID_SL_LCD, PID_STRIMER,
    PID_TLV2_LCD, PID_TX, PID_UNIVERSAL_88, VID_AURA, VID_HUB, VID_LCD, VID_LIANLI, VID_STRIMER,
};

// USB endpoints
pub const EP_OUT: u8 = 0x01;
pub const EP_IN: u8 = 0x81;

// Command bytes
pub const CMD_QUERY: u8 = 0x10;
pub const CMD_TX_SYNC: u8 = 0x11;
pub const CMD_TYPE_SPEED: [u8; 2] = [0x12, 0x10];
pub const CMD_TYPE_RING_SELECT: [u8; 2] = [0x12, 0x12];
pub const CMD_TYPE_MASTER_CLOCK: [u8; 2] = [0x12, 0x14];
pub const CMD_TYPE_BIND: [u8; 2] = [0x12, 0x15];
pub const CMD_TYPE_REBIND: [u8; 2] = [0x12, 0x19];
pub const CMD_TYPE_RGB: [u8; 2] = [0x12, 0x20];
pub const CMD_TYPE_AIO_INFO: [u8; 2] = [0x12, 0x21];
pub const CMD_TYPE_MB_SYNC: [u8; 2] = [0x12, 0x24];

// RF query/control commands
pub const RF_CMD_GET_GROUP_NUM: u8 = 0x01;
pub const RF_CMD_GET_RPM: u8 = 0x02;
pub const RF_CMD_GET_ERROR: u8 = 0x03;
pub const RF_CMD_SET_FAN_GROUP: u8 = 0x12;
pub const RF_CMD_LCD_RESET: u8 = 0x15;
pub const RF_CMD_SET_LE_DIRECTION: u8 = 0x16;
pub const RF_CMD_SET_ORDER: u8 = 0x19;

// Speed byte range
pub const SPEED_MIN: u8 = 0x06;
pub const SPEED_MAX: u8 = 0xFF;

// Timing (milliseconds)
pub const DELAY_COMMAND_MS: u64 = 15;
pub const DELAY_SETUP_MS: u64 = 20;
pub const DELAY_SYNC_MS: u64 = 50;

// Default wireless channel
pub const DEFAULT_CHANNEL: u8 = 0x08;

/// Valid Lian Li RF channels. L-Connect uses 0x01..=0x0F.
pub const VALID_CHANNELS: [u8; 15] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
];

// Broadcast group
pub const GROUP_BROADCAST: u8 = 0xFF;

// RGB max value (protocol uses 254, not 255)
pub const RGB_PROTOCOL_MAX: u8 = 0xFE;

// Brightness levels
pub const BRIGHTNESS_LEVELS: [u8; 5] = [0, 64, 128, 192, 255];

// Effect speed levels (inverted: L1=slowest=7, L5=fastest=3)
pub const EFFECT_SPEED_LEVELS: [u8; 5] = [7, 6, 5, 4, 3];
