//! IPC server — Unix domain socket listener for the daemon.

use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use frgb_model::ipc::{Event, Request, Response};

pub use frgb_ipc::socket_path;

/// IPC server that accepts client connections.
pub struct IpcServer {
    listener: UnixListener,
    path: PathBuf,
}

impl IpcServer {
    pub fn bind(path: &Path) -> io::Result<Self> {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let listener = UnixListener::bind(path)?;
        // Restrict socket to owner only (0600)
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        listener.set_nonblocking(true)?;
        tracing::info!("IPC listening on {}", path.display());
        Ok(Self {
            listener,
            path: path.to_owned(),
        })
    }

    pub fn accept(&self) -> Option<IpcConnection> {
        match self.listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).ok();
                stream.set_read_timeout(Some(Duration::from_millis(50))).ok();
                Some(IpcConnection {
                    stream,
                    subscribed: false,
                })
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => None,
            Err(e) => {
                tracing::warn!("IPC accept error: {e}");
                None
            }
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}

/// A single client connection (server side).
pub struct IpcConnection {
    stream: UnixStream,
    /// Whether this client has sent a Subscribe request.
    /// Only subscribed clients receive event broadcasts.
    pub subscribed: bool,
}

impl IpcConnection {
    pub fn read_request(&mut self) -> std::io::Result<Option<Request>> {
        match frgb_ipc::read_framed(&mut self.stream) {
            None => Ok(None),
            Some(Ok(req)) => Ok(Some(req)),
            Some(Err(e)) => Err(e),
        }
    }

    pub fn send_response(&mut self, response: &Response) -> io::Result<()> {
        frgb_ipc::write_framed(&mut self.stream, response)
    }

    pub fn send_event(&mut self, event: &Event) -> io::Result<()> {
        frgb_ipc::write_framed(&mut self.stream, &Response::Event(event.clone()))
    }
}
