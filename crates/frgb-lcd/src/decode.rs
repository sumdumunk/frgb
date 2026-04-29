use crate::{CMD_INIT, CMD_INIT_FINAL, CMD_PUSH_JPG, RESP_SUCCESS};

/// Parsed image acknowledgment.
#[derive(Debug, Clone, Copy)]
pub struct ImageAck {
    pub sequence: u16,
}

/// Validate a command response. LCD protocol responses echo the command byte
/// in byte[0] with RESP_SUCCESS (0xc8) in byte[1].
pub fn validate_cmd_response(expected_cmd: u8, data: &[u8]) -> Result<(), String> {
    if data.len() < 2 {
        return Err(format!("response too short: {} bytes", data.len()));
    }
    if data[0] != expected_cmd || data[1] != RESP_SUCCESS {
        return Err(format!(
            "expected response {:02x} {:02x}, got {:02x} {:02x}",
            expected_cmd, RESP_SUCCESS, data[0], data[1],
        ));
    }
    Ok(())
}

/// Validate an init response (expected: `[CMD_INIT, RESP_SUCCESS, ...]`).
pub fn validate_init_response(data: &[u8]) -> Result<(), String> {
    validate_cmd_response(CMD_INIT, data)
}

/// Validate a final init response (expected: `[CMD_INIT_FINAL, RESP_SUCCESS, ...]`).
pub fn validate_final_init_response(data: &[u8]) -> Result<(), String> {
    validate_cmd_response(CMD_INIT_FINAL, data)
}

/// Validate an image acknowledgment (expected: `[CMD_PUSH_JPG, RESP_SUCCESS, seq_lo, seq_hi, ...]`).
pub fn validate_image_ack(data: &[u8]) -> Result<ImageAck, String> {
    if data.len() < 4 {
        return Err(format!("response too short: {} bytes", data.len()));
    }
    validate_cmd_response(CMD_PUSH_JPG, data)?;
    let sequence = u16::from_le_bytes([data[2], data[3]]);
    Ok(ImageAck { sequence })
}

/// Parse temperature payload from a validated GetTemperature response.
///
/// Caller must validate the response header first via `validate_cmd_response()`.
/// Response format is unverified against real hardware — needs hardware testing.
/// Returns None if the device reports no sensor data (zero payload).
pub fn parse_temperature(data: &[u8]) -> Result<Option<f32>, String> {
    if data.len() < 4 {
        return Err(format!("temperature response too short: {} bytes", data.len()));
    }
    // Bytes 2-3: temperature data. Zero = no sensor.
    if data[2] == 0 && data[3] == 0 {
        return Ok(None);
    }
    // Unverified: treating byte[2] as integer °C, byte[3] as tenths.
    // Needs hardware validation — may require different interpretation.
    let temp = data[2] as f32 + data[3] as f32 / 10.0;
    Ok(Some(temp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CMD_GET_TEMPERATURE;

    #[test]
    fn validate_init_response_ok() {
        let mut resp = [0u8; 512];
        resp[0] = CMD_INIT;
        resp[1] = RESP_SUCCESS;
        assert!(validate_init_response(&resp).is_ok());
    }

    #[test]
    fn validate_init_response_wrong_header() {
        let mut resp = [0u8; 512];
        resp[0] = CMD_INIT;
        resp[1] = 0x00;
        assert!(validate_init_response(&resp).is_err());
    }

    #[test]
    fn validate_init_response_too_short() {
        let resp = [CMD_INIT];
        assert!(validate_init_response(&resp).is_err());
    }

    #[test]
    fn validate_final_init_response_ok() {
        let mut resp = [0u8; 512];
        resp[0] = CMD_INIT_FINAL;
        resp[1] = RESP_SUCCESS;
        assert!(validate_final_init_response(&resp).is_ok());
    }

    #[test]
    fn validate_final_init_response_wrong_cmd() {
        let mut resp = [0u8; 512];
        resp[0] = CMD_INIT;
        resp[1] = RESP_SUCCESS;
        assert!(validate_final_init_response(&resp).is_err());
    }

    #[test]
    fn validate_image_ack_ok() {
        let mut resp = [0u8; 512];
        resp[0] = CMD_PUSH_JPG;
        resp[1] = RESP_SUCCESS;
        resp[2] = 0x00;
        resp[3] = 0x10;
        let ack = validate_image_ack(&resp).unwrap();
        assert_eq!(ack.sequence, 0x1000);
    }

    #[test]
    fn validate_image_ack_wrong_header() {
        let mut resp = [0u8; 512];
        resp[0] = CMD_PUSH_JPG;
        resp[1] = 0x00;
        assert!(validate_image_ack(&resp).is_err());
    }

    #[test]
    fn validate_image_ack_too_short() {
        let resp = [CMD_PUSH_JPG, RESP_SUCCESS, 0x00];
        assert!(validate_image_ack(&resp).is_err());
    }

    #[test]
    fn validate_cmd_response_ok() {
        let mut resp = [0u8; 64];
        resp[0] = 0x61;
        resp[1] = RESP_SUCCESS;
        assert!(validate_cmd_response(0x61, &resp).is_ok());
    }

    #[test]
    fn validate_cmd_response_wrong_cmd() {
        let mut resp = [0u8; 64];
        resp[0] = 0x60;
        resp[1] = RESP_SUCCESS;
        assert!(validate_cmd_response(0x61, &resp).is_err());
    }

    #[test]
    fn validate_cmd_response_wrong_status() {
        let mut resp = [0u8; 64];
        resp[0] = 0x61;
        resp[1] = 0x00;
        assert!(validate_cmd_response(0x61, &resp).is_err());
    }

    #[test]
    fn parse_temperature_valid() {
        let mut resp = [0u8; 64];
        resp[0] = CMD_GET_TEMPERATURE;
        resp[1] = RESP_SUCCESS;
        resp[2] = 35;
        resp[3] = 5;
        let temp = parse_temperature(&resp).unwrap();
        assert!((temp.unwrap() - 35.5).abs() < 0.01);
    }

    #[test]
    fn parse_temperature_no_sensor() {
        let mut resp = [0u8; 64];
        resp[0] = CMD_GET_TEMPERATURE;
        resp[1] = RESP_SUCCESS;
        resp[2] = 0;
        resp[3] = 0;
        assert!(parse_temperature(&resp).unwrap().is_none());
    }

    #[test]
    fn parse_temperature_too_short() {
        let resp = [CMD_GET_TEMPERATURE, RESP_SUCCESS, 35];
        assert!(parse_temperature(&resp).is_err());
    }
}
