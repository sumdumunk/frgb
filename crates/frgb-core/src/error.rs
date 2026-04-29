use std::fmt;

#[derive(Debug)]
pub enum CoreError {
    Usb(frgb_usb::error::UsbError),
    Protocol(String),
    Config(String),
    NotFound(String),
    InvalidInput(String),
    NotSupported(String),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usb(e) => write!(f, "USB error: {e}"),
            Self::Protocol(msg) => write!(f, "protocol error: {msg}"),
            Self::Config(msg) => write!(f, "config error: {msg}"),
            Self::NotFound(msg) => write!(f, "not found: {msg}"),
            Self::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
            Self::NotSupported(msg) => write!(f, "not supported: {msg}"),
        }
    }
}

impl std::error::Error for CoreError {}

impl From<frgb_usb::error::UsbError> for CoreError {
    fn from(e: frgb_usb::error::UsbError) -> Self {
        Self::Usb(e)
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;
