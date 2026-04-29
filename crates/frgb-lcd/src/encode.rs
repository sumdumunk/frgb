use crate::encrypt::encrypt_packet;
use crate::{
    CMD_GET_H2_PARAMS, CMD_GET_TEMPERATURE, CMD_GET_VER, CMD_INIT, CMD_INIT_FINAL, CMD_PUSH_JPG, CMD_QUERY_BLOCK,
    CMD_REBOOT, CMD_SET_CLOCK, CMD_SET_FRAMERATE, CMD_SET_WTHEME_INDEX, CMD_START_PLAY, CMD_STOP_CLOCK, CMD_STOP_PLAY,
    IMAGE_HEADER_SIZE, IMAGE_PACKET_SIZE, INIT_DELAYS, MAGIC_1, MAGIC_2, MAX_JPEG_SIZE, PACKET_SIZE, PLAINTEXT_SIZE,
    RESP_SUCCESS,
};
use frgb_model::lcd::LcdRotation;

/// Build a single encrypted init/command packet.
///
/// Layout:
/// - byte[0] = command
/// - byte[2] = 0x1a, byte[3] = 0x6d (magic)
/// - byte[4..8] = timestamp (little-endian milliseconds)
/// - byte[8] = data byte (delay value, brightness, rotation, etc.)
pub fn build_init_packet(cmd: u8, timestamp_ms: u32, data_byte: u8) -> [u8; PACKET_SIZE] {
    let mut plaintext = [0u8; PLAINTEXT_SIZE];
    plaintext[0] = cmd;
    plaintext[2] = MAGIC_1;
    plaintext[3] = MAGIC_2;
    plaintext[4..8].copy_from_slice(&timestamp_ms.to_le_bytes());
    plaintext[8] = data_byte;
    encrypt_packet(&plaintext)
}

/// Build the complete 25-packet initialization sequence.
///
/// Returns 25 encrypted packets:
/// - 24 init packets (12 delay values × 2 sends each)
/// - 1 final init packet (CMD_INIT_FINAL)
///
/// `base_ms` is the starting timestamp; each packet increments by 5ms.
pub fn build_init_sequence(base_ms: u32) -> Vec<[u8; PACKET_SIZE]> {
    let mut packets = Vec::with_capacity(25);
    let mut ts = base_ms;

    for &delay in &INIT_DELAYS {
        for _ in 0..2 {
            packets.push(build_init_packet(CMD_INIT, ts, delay));
            ts = ts.wrapping_add(5);
        }
    }

    packets.push(build_init_packet(CMD_INIT_FINAL, ts, 0));
    packets
}

/// Build an encrypted 512-byte image header.
///
/// cmd=0x65, image size in bytes 8-11 big-endian.
pub fn build_image_header(timestamp_ms: u32, jpeg_size: u32) -> [u8; PACKET_SIZE] {
    let mut plaintext = [0u8; PLAINTEXT_SIZE];
    plaintext[0] = CMD_PUSH_JPG;
    plaintext[2] = MAGIC_1;
    plaintext[3] = MAGIC_2;
    plaintext[4..8].copy_from_slice(&timestamp_ms.to_le_bytes());
    plaintext[8..12].copy_from_slice(&jpeg_size.to_be_bytes());
    encrypt_packet(&plaintext)
}

/// Build a 512-byte image header using WinUSB encryption (HydroShift, Universal).
pub fn build_image_header_winusb(timestamp_ms: u32, jpeg_size: u32) -> [u8; PACKET_SIZE] {
    let mut plaintext = [0u8; 500];
    plaintext[0] = CMD_PUSH_JPG;
    plaintext[2] = MAGIC_1;
    plaintext[3] = MAGIC_2;
    plaintext[4..8].copy_from_slice(&timestamp_ms.to_le_bytes());
    plaintext[8..12].copy_from_slice(&jpeg_size.to_be_bytes());
    crate::encrypt::encrypt_packet_winusb(&plaintext)
}

/// Build a complete 102,400-byte image packet.
///
/// Layout: 512-byte encrypted header + raw JPEG data + zero padding.
/// Returns an error if JPEG exceeds MAX_JPEG_SIZE (101,888 bytes).
pub fn build_image_packet(timestamp_ms: u32, jpeg: &[u8]) -> Result<Vec<u8>, String> {
    if jpeg.len() > MAX_JPEG_SIZE {
        return Err(format!("JPEG too large: {} bytes, max {}", jpeg.len(), MAX_JPEG_SIZE,));
    }
    let header = build_image_header(timestamp_ms, jpeg.len() as u32);

    let mut packet = vec![0u8; IMAGE_PACKET_SIZE];
    packet[..IMAGE_HEADER_SIZE].copy_from_slice(&header);
    packet[IMAGE_HEADER_SIZE..IMAGE_HEADER_SIZE + jpeg.len()].copy_from_slice(jpeg);
    Ok(packet)
}

/// Build a complete 102,400-byte image packet using WinUSB encryption.
///
/// Same layout as standard, but header uses WinUSB encryption format.
pub fn build_image_packet_winusb(timestamp_ms: u32, jpeg: &[u8]) -> Result<Vec<u8>, String> {
    if jpeg.len() > MAX_JPEG_SIZE {
        return Err(format!("JPEG too large: {} bytes, max {}", jpeg.len(), MAX_JPEG_SIZE,));
    }
    let header = build_image_header_winusb(timestamp_ms, jpeg.len() as u32);

    let mut packet = vec![0u8; IMAGE_PACKET_SIZE];
    packet[..IMAGE_HEADER_SIZE].copy_from_slice(&header);
    packet[IMAGE_HEADER_SIZE..IMAGE_HEADER_SIZE + jpeg.len()].copy_from_slice(jpeg);
    Ok(packet)
}

/// Build a 512-byte control packet sent after each image.
///
/// This packet is NOT encrypted — it's sent raw.
/// Format: [0x65, 0xc8, seq_lo, seq_hi, device_id, 0x0b, ...zeros]
pub fn build_control_packet(sequence: u16, device_id: u8) -> [u8; PACKET_SIZE] {
    let mut packet = [0u8; PACKET_SIZE];
    packet[0] = CMD_PUSH_JPG;
    packet[1] = RESP_SUCCESS;
    packet[2] = (sequence & 0xFF) as u8;
    packet[3] = ((sequence >> 8) & 0xFF) as u8;
    packet[4] = device_id;
    packet[5] = 0x0b;
    packet
}

/// Build an encrypted brightness command.
///
/// The caller is responsible for the /2 division if mapping from 0-100%.
/// `brightness` is the raw byte value (0-50).
pub fn build_brightness_packet(timestamp_ms: u32, brightness: u8) -> Result<[u8; PACKET_SIZE], String> {
    if brightness > 50 {
        return Err(format!("brightness out of range [0-50]: {brightness}"));
    }
    Ok(build_init_packet(CMD_INIT, timestamp_ms, brightness))
}

/// Build an encrypted rotation command.
pub fn build_rotation_packet(timestamp_ms: u32, rotation: LcdRotation) -> [u8; PACKET_SIZE] {
    let rotation_byte = match rotation {
        LcdRotation::R0 => 0,
        LcdRotation::R90 => 1,
        LcdRotation::R180 => 2,
        LcdRotation::R270 => 3,
    };
    build_init_packet(CMD_INIT_FINAL, timestamp_ms, rotation_byte)
}

/// Build an encrypted frame rate command.
pub fn build_framerate_packet(timestamp_ms: u32, fps: u8) -> [u8; PACKET_SIZE] {
    build_init_packet(CMD_SET_FRAMERATE, timestamp_ms, fps)
}

/// Build an encrypted temperature query.
pub fn build_get_temperature_packet(timestamp_ms: u32) -> [u8; PACKET_SIZE] {
    build_init_packet(CMD_GET_TEMPERATURE, timestamp_ms, 0)
}

// ---------------------------------------------------------------------------
// Phase 6.2: Additional CmdType encoders
// ---------------------------------------------------------------------------

/// Query firmware version. Response: 32-byte UTF-8 string at bytes 8-39.
pub fn build_get_ver_packet(timestamp_ms: u32) -> [u8; PACKET_SIZE] {
    build_init_packet(CMD_GET_VER, timestamp_ms, 0)
}

/// Reboot the LCD device. No response expected.
pub fn build_reboot_packet(timestamp_ms: u32) -> [u8; PACKET_SIZE] {
    build_init_packet(CMD_REBOOT, timestamp_ms, 0)
}

/// Set system clock on the LCD device.
///
/// 8 bytes of time data starting at byte[8].
/// Format: year(2), month, day, hour, minute, second, weekday.
#[allow(clippy::too_many_arguments)] // clock packets naturally require year/month/day/hour/min/sec/weekday/style
pub fn build_set_clock_packet(
    timestamp_ms: u32,
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    weekday: u8,
) -> [u8; PACKET_SIZE] {
    let mut plaintext = [0u8; PLAINTEXT_SIZE];
    plaintext[0] = CMD_SET_CLOCK;
    plaintext[2] = MAGIC_1;
    plaintext[3] = MAGIC_2;
    plaintext[4..8].copy_from_slice(&timestamp_ms.to_le_bytes());
    plaintext[8] = (year >> 8) as u8;
    plaintext[9] = (year & 0xFF) as u8;
    plaintext[10] = month;
    plaintext[11] = day;
    plaintext[12] = hour;
    plaintext[13] = minute;
    plaintext[14] = second;
    plaintext[15] = weekday;
    encrypt_packet(&plaintext)
}

/// Stop clock display on the LCD.
pub fn build_stop_clock_packet(timestamp_ms: u32) -> [u8; PACKET_SIZE] {
    build_init_packet(CMD_STOP_CLOCK, timestamp_ms, 0)
}

/// Start H.264 video playback. `block_id` identifies the video file.
pub fn build_start_play_packet(timestamp_ms: u32, block_id: u8) -> [u8; PACKET_SIZE] {
    build_init_packet(CMD_START_PLAY, timestamp_ms, block_id)
}

/// Stop video playback.
pub fn build_stop_play_packet(timestamp_ms: u32) -> [u8; PACKET_SIZE] {
    build_init_packet(CMD_STOP_PLAY, timestamp_ms, 0)
}

/// Query playback block status. Response: block counts at bytes 8-10.
pub fn build_query_block_packet(timestamp_ms: u32) -> [u8; PACKET_SIZE] {
    build_init_packet(CMD_QUERY_BLOCK, timestamp_ms, 0)
}

/// Select a theme/preset by index.
pub fn build_set_theme_index_packet(timestamp_ms: u32, theme_index: u8) -> [u8; PACKET_SIZE] {
    build_init_packet(CMD_SET_WTHEME_INDEX, timestamp_ms, theme_index)
}

/// Query HydroShift II AIO-specific parameters.
pub fn build_get_h2_params_packet(timestamp_ms: u32) -> [u8; PACKET_SIZE] {
    build_init_packet(CMD_GET_H2_PARAMS, timestamp_ms, 0)
}

/// Build a WinUSB-format encrypted command packet.
///
/// Used by HydroShift II, Lancool 207, Universal Screen.
/// 500-byte plaintext → PKCS7 → 504 ciphertext + trailers at bytes 510-511.
pub fn build_winusb_packet(cmd: u8, timestamp_ms: u32, data_byte: u8) -> [u8; PACKET_SIZE] {
    let mut plaintext = [0u8; 500];
    plaintext[0] = cmd;
    plaintext[2] = MAGIC_1;
    plaintext[3] = MAGIC_2;
    plaintext[4..8].copy_from_slice(&timestamp_ms.to_le_bytes());
    plaintext[8] = data_byte;
    crate::encrypt::encrypt_packet_winusb(&plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encrypt::decrypt_packet;
    use crate::{
        CMD_GET_H2_PARAMS, CMD_GET_VER, CMD_INIT, CMD_INIT_FINAL, CMD_PUSH_JPG, CMD_QUERY_BLOCK, CMD_REBOOT,
        CMD_SET_CLOCK, CMD_SET_FRAMERATE, CMD_SET_WTHEME_INDEX, CMD_START_PLAY, CMD_STOP_CLOCK, CMD_STOP_PLAY,
        IMAGE_HEADER_SIZE, IMAGE_PACKET_SIZE, INIT_DELAYS, MAGIC_1, MAGIC_2, MAX_JPEG_SIZE, PACKET_SIZE,
    };

    #[test]
    fn build_init_packet_structure() {
        let packet = build_init_packet(CMD_INIT, 1000, 0x64);
        assert_eq!(packet.len(), PACKET_SIZE);
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_INIT);
        assert_eq!(plain[1], 0x00);
        assert_eq!(plain[2], MAGIC_1);
        assert_eq!(plain[3], MAGIC_2);
        let ts = u32::from_le_bytes([plain[4], plain[5], plain[6], plain[7]]);
        assert_eq!(ts, 1000);
        assert_eq!(plain[8], 0x64);
    }

    #[test]
    fn build_final_init_packet_structure() {
        let packet = build_init_packet(CMD_INIT_FINAL, 5000, 0);
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_INIT_FINAL);
        assert_eq!(plain[8], 0);
    }

    #[test]
    fn build_init_sequence_length() {
        let packets = build_init_sequence(0);
        assert_eq!(packets.len(), 25);
        for p in &packets {
            assert_eq!(p.len(), PACKET_SIZE);
        }
    }

    #[test]
    fn init_sequence_delay_values_match() {
        let packets = build_init_sequence(0);
        for (i, delay) in INIT_DELAYS.iter().enumerate() {
            let p1 = decrypt_packet(&packets[i * 2]).unwrap();
            let p2 = decrypt_packet(&packets[i * 2 + 1]).unwrap();
            assert_eq!(p1[0], CMD_INIT);
            assert_eq!(p2[0], CMD_INIT);
            assert_eq!(p1[8], *delay, "pair {} first packet delay", i);
            assert_eq!(p2[8], *delay, "pair {} second packet delay", i);
        }
        let final_p = decrypt_packet(&packets[24]).unwrap();
        assert_eq!(final_p[0], CMD_INIT_FINAL);
    }

    #[test]
    fn init_sequence_timestamps_advance() {
        let packets = build_init_sequence(1000);
        let p0 = decrypt_packet(&packets[0]).unwrap();
        let p1 = decrypt_packet(&packets[1]).unwrap();
        let ts0 = u32::from_le_bytes([p0[4], p0[5], p0[6], p0[7]]);
        let ts1 = u32::from_le_bytes([p1[4], p1[5], p1[6], p1[7]]);
        assert_eq!(ts1, ts0 + 5);
    }

    #[test]
    fn build_image_header_structure() {
        let jpeg_size: u32 = 50_000;
        let header = build_image_header(2000, jpeg_size);
        assert_eq!(header.len(), PACKET_SIZE);
        let plain = decrypt_packet(&header).unwrap();
        assert_eq!(plain[0], CMD_PUSH_JPG);
        assert_eq!(plain[2], MAGIC_1);
        assert_eq!(plain[3], MAGIC_2);
        let ts = u32::from_le_bytes([plain[4], plain[5], plain[6], plain[7]]);
        assert_eq!(ts, 2000);
        let size = u32::from_be_bytes([plain[8], plain[9], plain[10], plain[11]]);
        assert_eq!(size, jpeg_size);
    }

    #[test]
    fn build_image_packet_total_size() {
        let jpeg = vec![0xFFu8; 1000];
        let packet = build_image_packet(3000, &jpeg).unwrap();
        assert_eq!(packet.len(), IMAGE_PACKET_SIZE);
    }

    #[test]
    fn build_image_packet_contains_jpeg() {
        let jpeg = vec![0xAB; 500];
        let packet = build_image_packet(3000, &jpeg).unwrap();
        assert_eq!(&packet[IMAGE_HEADER_SIZE..IMAGE_HEADER_SIZE + 500], &[0xAB; 500]);
        assert!(packet[IMAGE_HEADER_SIZE + 500..].iter().all(|&b| b == 0));
    }

    #[test]
    fn build_image_packet_rejects_oversized_jpeg() {
        let jpeg = vec![0xCC; MAX_JPEG_SIZE + 1000];
        let result = build_image_packet(3000, &jpeg);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("JPEG too large"));
    }

    #[test]
    fn build_control_packet_structure() {
        let packet = build_control_packet(0x1000, 0x5a);
        assert_eq!(packet.len(), PACKET_SIZE);
        assert_eq!(packet[0], 0x65);
        assert_eq!(packet[1], 0xc8);
        assert_eq!(packet[2], 0x00); // 0x1000 & 0xFF
        assert_eq!(packet[3], 0x10); // (0x1000 >> 8) & 0xFF
        assert_eq!(packet[4], 0x5a);
        assert_eq!(packet[5], 0x0b);
        assert!(packet[6..].iter().all(|&b| b == 0));
    }

    #[test]
    fn build_brightness_packet_structure() {
        let packet = build_brightness_packet(1000, 25).unwrap();
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_INIT); // CmdType::BrigthSet = 0x0e
        assert_eq!(plain[8], 25);
    }

    #[test]
    fn build_brightness_rejects_out_of_range() {
        assert!(build_brightness_packet(1000, 51).is_err());
        assert!(build_brightness_packet(1000, 50).is_ok());
    }

    #[test]
    fn build_rotation_packet_values() {
        use frgb_model::lcd::LcdRotation;

        for (rotation, expected_byte) in [
            (LcdRotation::R0, 0),
            (LcdRotation::R90, 1),
            (LcdRotation::R180, 2),
            (LcdRotation::R270, 3),
        ] {
            let packet = build_rotation_packet(1000, rotation);
            let plain = decrypt_packet(&packet).unwrap();
            assert_eq!(plain[0], CMD_INIT_FINAL); // CmdType::Rotate = 0x0d
            assert_eq!(plain[8], expected_byte);
        }
    }

    #[test]
    fn build_framerate_packet_structure() {
        let packet = build_framerate_packet(1000, 30);
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_SET_FRAMERATE);
        assert_eq!(plain[8], 30);
    }

    #[test]
    fn build_get_temperature_packet_structure() {
        let packet = build_get_temperature_packet(3000);
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_GET_TEMPERATURE);
        assert_eq!(plain[8], 0);
    }

    // --- Phase 6.2 encoder tests ---

    #[test]
    fn get_ver_packet_cmd() {
        let packet = super::build_get_ver_packet(1000);
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_GET_VER);
    }

    #[test]
    fn reboot_packet_cmd() {
        let packet = super::build_reboot_packet(1000);
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_REBOOT);
    }

    #[test]
    fn set_clock_packet_data() {
        let packet = super::build_set_clock_packet(1000, 2026, 3, 28, 14, 30, 0, 6);
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_SET_CLOCK);
        assert_eq!(plain[8], 0x07); // 2026 >> 8
        assert_eq!(plain[9], 0xEA); // 2026 & 0xFF
        assert_eq!(plain[10], 3); // month
        assert_eq!(plain[11], 28); // day
        assert_eq!(plain[12], 14); // hour
        assert_eq!(plain[13], 30); // minute
        assert_eq!(plain[14], 0); // second
        assert_eq!(plain[15], 6); // weekday (Saturday)
    }

    #[test]
    fn stop_clock_packet_cmd() {
        let packet = super::build_stop_clock_packet(1000);
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_STOP_CLOCK);
    }

    #[test]
    fn start_play_packet_block_id() {
        let packet = super::build_start_play_packet(1000, 3);
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_START_PLAY);
        assert_eq!(plain[8], 3);
    }

    #[test]
    fn stop_play_packet_cmd() {
        let packet = super::build_stop_play_packet(1000);
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_STOP_PLAY);
    }

    #[test]
    fn query_block_packet_cmd() {
        let packet = super::build_query_block_packet(1000);
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_QUERY_BLOCK);
    }

    #[test]
    fn set_theme_index_packet_data() {
        let packet = super::build_set_theme_index_packet(1000, 5);
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_SET_WTHEME_INDEX);
        assert_eq!(plain[8], 5);
    }

    #[test]
    fn get_h2_params_packet_cmd() {
        let packet = super::build_get_h2_params_packet(1000);
        let plain = decrypt_packet(&packet).unwrap();
        assert_eq!(plain[0], CMD_GET_H2_PARAMS);
    }
}
