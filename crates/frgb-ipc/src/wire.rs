use std::io::{self, Read, Write};

/// Maximum IPC message size (2 MiB — local socket, preset thumbnails can be large).
pub const IPC_MAX_MESSAGE_SIZE: usize = 2 * 1024 * 1024;

/// Write a length-prefixed JSON frame to a stream.
pub fn write_framed<T: serde::Serialize>(stream: &mut impl Write, msg: &T) -> io::Result<()> {
    let json = serde_json::to_vec(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = json.len() as u32;
    stream.write_all(&len.to_le_bytes())?;
    stream.write_all(&json)?;
    stream.flush()
}

/// Read a length-prefixed JSON frame from a stream.
///
/// Returns `None` on EOF, `WouldBlock`, or `TimedOut`.
/// Returns `Some(Err(...))` on malformed data or oversized messages.
/// Returns `Some(Ok(msg))` on success.
pub fn read_framed<T: serde::de::DeserializeOwned>(stream: &mut impl Read) -> Option<io::Result<T>> {
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => return None,
        Err(e) if e.kind() == io::ErrorKind::TimedOut => return None,
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return None,
        Err(e) => return Some(Err(e)),
    }

    let len = u32::from_le_bytes(len_buf) as usize;
    if len > IPC_MAX_MESSAGE_SIZE {
        return Some(Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("IPC message too large: {len} bytes (max {IPC_MAX_MESSAGE_SIZE})"),
        )));
    }

    let mut buf = vec![0u8; len];
    if let Err(e) = stream.read_exact(&mut buf) {
        return Some(Err(e));
    }

    match serde_json::from_slice(&buf) {
        Ok(msg) => Some(Ok(msg)),
        Err(e) => Some(Err(io::Error::new(io::ErrorKind::InvalidData, e))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn roundtrip_string() {
        let mut buf = Vec::new();
        write_framed(&mut buf, &"hello").unwrap();

        let mut cursor = Cursor::new(buf);
        let result: String = read_framed(&mut cursor).unwrap().unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn roundtrip_request() {
        use frgb_model::ipc::Request;
        let req = Request::Status;

        let mut buf = Vec::new();
        write_framed(&mut buf, &req).unwrap();

        let mut cursor = Cursor::new(buf);
        let result: Request = read_framed(&mut cursor).unwrap().unwrap();
        assert_eq!(format!("{result:?}"), "Status");
    }

    #[test]
    fn oversized_message_rejected() {
        let len = (IPC_MAX_MESSAGE_SIZE as u32 + 1).to_le_bytes();
        let mut cursor = Cursor::new(len.to_vec());
        let result: Option<io::Result<String>> = read_framed(&mut cursor);
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn eof_returns_none() {
        let mut cursor = Cursor::new(vec![0u8; 2]); // incomplete length
        let result: Option<io::Result<String>> = read_framed(&mut cursor);
        assert!(result.is_none());
    }

    #[test]
    fn roundtrip_request_with_newtypes() {
        use frgb_model::ipc::Request;
        use frgb_model::speed::SpeedMode;
        use frgb_model::GroupId;
        use frgb_model::SpeedPercent;

        let req = Request::SetSpeed {
            group: GroupId::new(3),
            mode: SpeedMode::Manual(SpeedPercent::new(80)),
        };

        let mut buf = Vec::new();
        write_framed(&mut buf, &req).unwrap();

        let mut cursor = Cursor::new(buf);
        let decoded: Request = read_framed(&mut cursor).unwrap().unwrap();

        match decoded {
            Request::SetSpeed { group, mode } => {
                assert_eq!(group, GroupId::new(3));
                assert_eq!(mode, SpeedMode::Manual(SpeedPercent::new(80)));
            }
            other => panic!("expected SetSpeed, got {other:?}"),
        }
    }
}
