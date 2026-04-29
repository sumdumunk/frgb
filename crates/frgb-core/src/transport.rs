use std::time::Duration;

/// USB packet size (64 bytes for fan hub protocol).
pub const PACKET_SIZE: usize = 64;

/// Abstraction over USB packet I/O. Real implementation wraps frgb_usb::UsbDevice.
/// Mock implementation records packets for testing.
pub trait Transport {
    fn write(&self, data: &[u8]) -> crate::error::Result<()>;
    fn read(&self, timeout: Duration) -> crate::error::Result<[u8; PACKET_SIZE]>;
    fn sleep(&self, duration: Duration);

    /// Attempt to reconnect the underlying transport. Default: no-op (success).
    fn reconnect(&mut self) -> crate::error::Result<()> {
        Ok(())
    }
}

/// Mock transport that records all writes and returns pre-configured responses.
/// Uses Mutex for interior mutability so MockTransport is Send+Sync.
#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    pub struct MockTransport {
        writes: Mutex<Vec<Vec<u8>>>,
        reads: Mutex<Vec<[u8; PACKET_SIZE]>>,
        sleeps: Mutex<Vec<Duration>>,
    }

    impl Default for MockTransport {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockTransport {
        pub fn new() -> Self {
            Self {
                writes: Mutex::new(Vec::new()),
                reads: Mutex::new(Vec::new()),
                sleeps: Mutex::new(Vec::new()),
            }
        }

        pub fn queue_read(&self, data: [u8; PACKET_SIZE]) {
            self.reads.lock().unwrap().push(data);
        }

        pub fn written_packets(&self) -> Vec<Vec<u8>> {
            self.writes.lock().unwrap().clone()
        }

        pub fn sleep_durations(&self) -> Vec<Duration> {
            self.sleeps.lock().unwrap().clone()
        }
    }

    impl Transport for MockTransport {
        fn write(&self, data: &[u8]) -> crate::error::Result<()> {
            self.writes.lock().unwrap().push(data.to_vec());
            Ok(())
        }

        fn read(&self, _timeout: Duration) -> crate::error::Result<[u8; PACKET_SIZE]> {
            let mut reads = self.reads.lock().unwrap();
            if reads.is_empty() {
                Err(crate::error::CoreError::Usb(frgb_usb::error::UsbError::Timeout))
            } else {
                Ok(reads.remove(0))
            }
        }

        fn sleep(&self, duration: Duration) {
            self.sleeps.lock().unwrap().push(duration);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockTransport;
    use super::*;

    #[test]
    fn mock_records_writes() {
        let mock = MockTransport::new();
        mock.write(&[0x10, 0x00]).unwrap();
        mock.write(&[0x10, 0x01]).unwrap();
        let packets = mock.written_packets();
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0], vec![0x10, 0x00]);
    }

    #[test]
    fn mock_returns_queued_reads() {
        let mock = MockTransport::new();
        let mut data = [0u8; PACKET_SIZE];
        data[0] = 0x11;
        data[1] = 0x08;
        mock.queue_read(data);
        let resp = mock.read(Duration::from_millis(100)).unwrap();
        assert_eq!(resp[0], 0x11);
        assert_eq!(resp[1], 0x08);
    }

    #[test]
    fn mock_read_empty_returns_timeout() {
        let mock = MockTransport::new();
        assert!(mock.read(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn mock_records_sleeps() {
        let mock = MockTransport::new();
        mock.sleep(Duration::from_millis(15));
        mock.sleep(Duration::from_millis(50));
        let sleeps = mock.sleep_durations();
        assert_eq!(sleeps.len(), 2);
        assert_eq!(sleeps[0], Duration::from_millis(15));
    }
}
