use rand::Rng;

/// Generate a random 16-bit session ID for RGB protocol commands.
///
/// L-Connect changes the session ID per invocation. Sending a stale/zero
/// value causes fans to ignore the command.
pub fn generate_session_id() -> u16 {
    rand::rng().random()
}

/// Generate a 4-byte effect index (change ID) for RGB effect transmission.
///
/// Must differ per send so the hardware knows this is a new effect,
/// not a repeat of the current one.
pub fn generate_effect_index() -> [u8; 4] {
    // Use random bytes to guarantee uniqueness per send.
    // L-Connect uses a timestamp; we use random bytes — firmware just needs a different value each send.
    rand::rng().random::<[u8; 4]>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_not_always_zero() {
        let ids: Vec<u16> = (0..100).map(|_| generate_session_id()).collect();
        assert!(ids.iter().any(|&id| id != 0));
    }

    #[test]
    fn session_ids_have_variance() {
        let ids: Vec<u16> = (0..100).map(|_| generate_session_id()).collect();
        let first = ids[0];
        assert!(ids.iter().any(|&id| id != first));
    }
}
