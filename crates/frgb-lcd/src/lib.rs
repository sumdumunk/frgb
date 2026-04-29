pub mod capture;
pub mod decode;
pub mod encode;
pub mod encrypt;
pub mod h264;
pub mod jpeg;
pub mod video;

// --- Protocol constants ---

/// DES key and IV: "slv3tuzx"
pub const DES_KEY: [u8; 8] = [115, 108, 118, 51, 116, 117, 122, 120];

/// Magic bytes in plaintext header (byte[2]=0x1a, byte[3]=0x6d)
pub const MAGIC_1: u8 = 0x1a;
pub const MAGIC_2: u8 = 0x6d;

/// Trailer bytes in encrypted packet (byte[510]=0xa1, byte[511]=0x1a)
pub const TRAILER_1: u8 = 0xa1;
pub const TRAILER_2: u8 = 0x1a;

/// Plaintext buffer size before encryption.
/// Standard LCD format: 504 bytes → PKCS7 → 512 bytes ciphertext.
pub const PLAINTEXT_SIZE: usize = 504;

/// Encrypted packet size (512 bytes — full PKCS7 ciphertext).
pub const PACKET_SIZE: usize = 512;

/// Ciphertext size for WinUSB format (500 + 4 zero-pad = 504, NoPadding).
pub const CIPHERTEXT_SIZE: usize = 504;

/// Total image packet size (encrypted header + raw JPEG + zero padding)
pub const IMAGE_PACKET_SIZE: usize = 102_400;

/// Encrypted header size within an image packet
pub const IMAGE_HEADER_SIZE: usize = 512;

/// Maximum raw JPEG payload size
pub const MAX_JPEG_SIZE: usize = IMAGE_PACKET_SIZE - IMAGE_HEADER_SIZE; // 101,888

// --- Command types ---

/// Init command (also used for brightness set)
pub const CMD_INIT: u8 = 0x0e;

/// Final init / rotate command
pub const CMD_INIT_FINAL: u8 = 0x0d;

/// Push JPEG image
pub const CMD_PUSH_JPG: u8 = 0x65;

/// Set frame rate
pub const CMD_SET_FRAMERATE: u8 = 0x0f;

/// Read coolant/block temperature (CmdType::GetTemperature = 96)
pub const CMD_GET_TEMPERATURE: u8 = 0x60;

// NOTE: CMD_SET_PUMP_SPEED (0x61) / CMD_GET_PUMP_SPEED (0x62) for the Lancool 207
// are NOT used for HydroShift II pumps. The HydroShift II pump is controlled via
// the Lian Li RF protocol (CMD_TYPE_AIO_INFO 0x12 0x21 in frgb-protocol), not via
// the LCD USB interface. These constants were intentionally removed — re-adding
// them for Lancool 207 support should happen in that backend's own module.

/// Get firmware version (CmdType::GetVer = 10)
pub const CMD_GET_VER: u8 = 0x0a;

/// Reboot LCD device (CmdType::Reboot = 11)
pub const CMD_REBOOT: u8 = 0x0b;

/// Set system clock (CmdType::SetClock = 51)
pub const CMD_SET_CLOCK: u8 = 0x33;

/// Stop clock display (CmdType::StopClock = 52)
pub const CMD_STOP_CLOCK: u8 = 0x34;

/// Start H.264 video playback (CmdType::StartPlay = 121)
pub const CMD_START_PLAY: u8 = 0x79;

/// Stop playback (CmdType::StopPlay = 123)
pub const CMD_STOP_PLAY: u8 = 0x7b;

/// Query playback block status (CmdType::QueryBlock = 122)
pub const CMD_QUERY_BLOCK: u8 = 0x7a;

/// Set theme/preset index (CmdType::SetWthemeIndex = 249)
pub const CMD_SET_WTHEME_INDEX: u8 = 0xf9;

/// Get H2 parameters (CmdType::GetH2Params = 250)
pub const CMD_GET_H2_PARAMS: u8 = 0xfa;

// --- Init sequence delay values (from captured USB data + Python reference) ---

/// The 12 delay values sent during LCD initialization.
/// Each value is placed at byte[8] of the init packet.
pub const INIT_DELAYS: [u8; 12] = [0x64, 0x28, 0x1e, 0x0a, 0x1e, 0x00, 0x0a, 0x1e, 0x28, 0x64, 0x64, 0x64];

// --- Response markers ---

/// Success status byte in LCD protocol responses (byte[1]).
pub const RESP_SUCCESS: u8 = 0xc8;

// --- Display resolutions ---

/// SL-LCD Wireless display resolution
pub const SL_LCD_WIDTH: u32 = 400;
pub const SL_LCD_HEIGHT: u32 = 400;

/// HydroShift LCD display resolution
pub const HYDROSHIFT_LCD_WIDTH: u32 = 480;
pub const HYDROSHIFT_LCD_HEIGHT: u32 = 480;
