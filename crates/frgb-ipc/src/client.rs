use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use frgb_model::ipc::{Request, Response};

use crate::wire::{read_framed, write_framed};

/// Synchronous IPC client for the frgb daemon.
pub struct IpcClient {
    stream: UnixStream,
}

impl IpcClient {
    /// Connect to the daemon at the given socket path.
    ///
    /// Sets a 10-second read timeout.
    pub fn connect(path: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        Ok(Self { stream })
    }

    /// Send a request and wait for the response.
    pub fn call(&mut self, request: &Request) -> io::Result<Response> {
        self.send(request)?;
        self.recv()
    }

    /// Send a request without waiting for a response.
    pub fn send(&mut self, request: &Request) -> io::Result<()> {
        write_framed(&mut self.stream, request)
    }

    /// Block until a response is received.
    ///
    /// Returns an error on EOF or deserialization failure.
    pub fn recv(&mut self) -> io::Result<Response> {
        match read_framed(&mut self.stream) {
            Some(Ok(response)) => Ok(response),
            Some(Err(e)) => Err(e),
            None => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "daemon closed connection")),
        }
    }

    /// Get a reference to the underlying stream (for cloning in EventStream).
    pub fn stream(&self) -> &UnixStream {
        &self.stream
    }
}
