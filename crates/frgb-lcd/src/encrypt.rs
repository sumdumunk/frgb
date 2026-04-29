use crate::{CIPHERTEXT_SIZE, DES_KEY, PACKET_SIZE, PLAINTEXT_SIZE, TRAILER_1, TRAILER_2};
use cbc::{
    cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit},
    Decryptor, Encryptor,
};
use des::Des;

type DesCbcEnc = Encryptor<Des>;
type DesCbcDec = Decryptor<Des>;

/// Encrypts a 504-byte plaintext into a 512-byte packet using PKCS7 padding.
///
/// This is the standard format used by VID=0x1CBE LCD devices (SLV3, TLV2,
/// HydroShift II). Matches the reference Rust project's `PacketBuilder::build()`.
///
/// 504 bytes plaintext → PKCS7 → 512 bytes ciphertext (full packet, no trailers).
pub fn encrypt_packet(plaintext: &[u8; PLAINTEXT_SIZE]) -> [u8; PACKET_SIZE] {
    // Need plaintext_size + block_size(8) bytes for encrypt_padded_mut
    let mut buf = [0u8; PLAINTEXT_SIZE + 8];
    buf[..PLAINTEXT_SIZE].copy_from_slice(plaintext);

    let enc = DesCbcEnc::new_from_slices(&DES_KEY, &DES_KEY).expect("DES key/IV are always valid 8-byte slices");
    let encrypted = enc
        .encrypt_padded_mut::<cbc::cipher::block_padding::Pkcs7>(&mut buf, PLAINTEXT_SIZE)
        .expect("PKCS7 padding of 504-byte buffer must produce 512 bytes");

    let mut packet = [0u8; PACKET_SIZE];
    packet.copy_from_slice(encrypted);
    packet
}

/// Encrypts using the WinUSB format (500-byte plaintext + NoPadding + trailers).
///
/// Used by wired USB hub devices. Not used by VID=0x1CBE LCD devices.
#[allow(dead_code)]
pub fn encrypt_packet_winusb(plaintext: &[u8; 500]) -> [u8; PACKET_SIZE] {
    let mut buf = [0u8; CIPHERTEXT_SIZE];
    buf[..500].copy_from_slice(plaintext);

    let enc = DesCbcEnc::new_from_slices(&DES_KEY, &DES_KEY).expect("DES key/IV are always valid 8-byte slices");
    enc.encrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut buf, CIPHERTEXT_SIZE)
        .expect("NoPadding encrypt of block-aligned buffer must not fail");

    let mut packet = [0u8; PACKET_SIZE];
    packet[..CIPHERTEXT_SIZE].copy_from_slice(&buf);
    packet[510] = TRAILER_1;
    packet[511] = TRAILER_2;
    packet
}

/// Decrypts a 512-byte packet back to plaintext.
///
/// Extracts the first CIPHERTEXT_SIZE (504) bytes and DES-CBC decrypts them.
/// Returns the 504-byte result (first PLAINTEXT_SIZE bytes are meaningful data;
/// the final 4 bytes are the zero-fill appended during encryption).
pub fn decrypt_packet(packet: &[u8]) -> Result<Vec<u8>, String> {
    if packet.len() < CIPHERTEXT_SIZE {
        return Err(format!(
            "packet too short: got {} bytes, need at least {}",
            packet.len(),
            CIPHERTEXT_SIZE
        ));
    }

    let mut buf = [0u8; CIPHERTEXT_SIZE];
    buf.copy_from_slice(&packet[..CIPHERTEXT_SIZE]);

    let dec = DesCbcDec::new_from_slices(&DES_KEY, &DES_KEY).expect("DES key/IV are always valid 8-byte slices");
    let decrypted = dec
        .decrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut buf)
        .map_err(|e| format!("DES-CBC decrypt failed: {e:?}"))?;

    Ok(decrypted.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CMD_INIT, MAGIC_1, MAGIC_2};

    #[test]
    fn encrypt_packet_size() {
        let plaintext = [0u8; PLAINTEXT_SIZE];
        let packet = encrypt_packet(&plaintext);
        assert_eq!(packet.len(), PACKET_SIZE);
    }

    #[test]
    fn encrypt_packet_full_512_ciphertext() {
        // Standard LCD format: 504 plaintext → PKCS7 → 512 bytes (no trailers)
        let plaintext = [0u8; PLAINTEXT_SIZE];
        let packet = encrypt_packet(&plaintext);
        assert_eq!(packet.len(), 512);
        // Last 8 bytes are PKCS7 encrypted padding, not zeros/trailers
        assert!(packet[504..].iter().any(|&b| b != 0));
    }

    #[test]
    fn encrypt_packet_ciphertext_not_zeros() {
        let mut plaintext = [0u8; PLAINTEXT_SIZE];
        plaintext[0] = CMD_INIT;
        plaintext[2] = MAGIC_1;
        plaintext[3] = MAGIC_2;
        let packet = encrypt_packet(&plaintext);
        assert!(packet.iter().any(|&b| b != 0));
    }

    #[test]
    fn decrypt_roundtrip() {
        let mut plaintext = [0u8; PLAINTEXT_SIZE];
        plaintext[0] = CMD_INIT;
        plaintext[2] = MAGIC_1;
        plaintext[3] = MAGIC_2;
        plaintext[8] = 0x64;
        let packet = encrypt_packet(&plaintext);
        let decrypted = decrypt_packet(&packet).unwrap();
        // Decrypted is 512 bytes; first 504 match plaintext, last 8 are PKCS7 pad
        assert_eq!(&decrypted[..PLAINTEXT_SIZE], &plaintext[..]);
    }

    #[test]
    fn decrypt_bad_length_fails() {
        let short = [0u8; 100];
        assert!(decrypt_packet(&short).is_err());
    }

    #[test]
    fn encrypt_winusb_matches_captured_init_000() {
        // Captured data is from L-Connect 3 (WinUSB format with trailers)
        let captured = include_bytes!("../../../external/from_windows/lcd_init_sequence/init_000.bin");
        let decrypted = decrypt_packet(captured).expect("captured packet should decrypt");
        let re_encrypted = encrypt_packet_winusb(&decrypted[..500].try_into().unwrap());
        assert_eq!(&re_encrypted[..CIPHERTEXT_SIZE], &captured[..CIPHERTEXT_SIZE]);
    }
}
