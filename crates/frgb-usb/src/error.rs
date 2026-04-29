use std::fmt;

#[derive(Debug)]
pub enum UsbError {
    NotFound,
    Busy,
    Permission,
    Timeout,
    Io(String),
}

impl fmt::Display for UsbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "USB device not found"),
            Self::Busy => write!(f, "USB device is busy (another process has it open)"),
            Self::Permission => write!(f, "USB permission denied (check udev rules)"),
            Self::Timeout => write!(f, "USB operation timed out"),
            Self::Io(msg) => write!(f, "USB IO error: {msg}"),
        }
    }
}

impl std::error::Error for UsbError {}

impl From<rusb::Error> for UsbError {
    fn from(e: rusb::Error) -> Self {
        match e {
            rusb::Error::NotFound => Self::NotFound,
            rusb::Error::Busy => Self::Busy,
            rusb::Error::Access => Self::Permission,
            rusb::Error::Timeout => Self::Timeout,
            other => Self::Io(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, UsbError>;
