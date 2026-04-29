mod client;
mod event_stream;
mod wire;

pub use client::IpcClient;
pub use event_stream::{run_event_loop, ConnectionCallback, EventCallback, EventStreamConfig, InitialStateCallback};
pub use wire::{read_framed, write_framed, IPC_MAX_MESSAGE_SIZE};

// Re-export IPC types so consumers don't need a direct frgb-model dependency for basic usage.
pub use frgb_model::ipc::{Event, Request, Response, Target, Topic, PROTOCOL_VERSION};

use std::path::PathBuf;

/// Returns the daemon socket path.
///
/// Uses `$XDG_RUNTIME_DIR/frgb.sock` if the directory exists, otherwise `/tmp/frgb.sock`.
pub fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        let path = PathBuf::from(&dir);
        if path.is_dir() {
            return path.join("frgb.sock");
        }
        tracing::warn!("XDG_RUNTIME_DIR={dir} does not exist, falling back to /tmp/");
    }
    PathBuf::from("/tmp/frgb.sock")
}

/// Returns true if the daemon is reachable via its socket.
pub fn daemon_running() -> bool {
    std::os::unix::net::UnixStream::connect(socket_path()).is_ok()
}
