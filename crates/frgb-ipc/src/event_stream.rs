use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use frgb_model::ipc::{Event, Request, Response, Topic, PROTOCOL_VERSION};
use tracing::{error, info, warn};

use crate::client::IpcClient;
use crate::wire::read_framed;

/// Callback type for received events.
pub type EventCallback = Box<dyn Fn(Event) + Send>;

/// Callback type for connection state changes.
pub type ConnectionCallback = Box<dyn Fn(bool) + Send>;

/// Callback type for initial state after (re)connect.
pub type InitialStateCallback = Box<dyn Fn(Response) + Send>;

/// Configuration for EventStream.
pub struct EventStreamConfig {
    pub socket_path: PathBuf,
    pub topics: Vec<Topic>,
    pub on_event: EventCallback,
    pub on_connection_change: ConnectionCallback,
    pub on_initial_state: InitialStateCallback,
}

/// Runs a blocking event loop that subscribes to daemon events and calls
/// the provided callback for each event. Reconnects automatically on
/// disconnect with exponential backoff.
///
/// This function blocks forever. Run it in a dedicated thread.
pub fn run_event_loop(config: EventStreamConfig) {
    let mut backoff = Duration::from_millis(100);
    let max_backoff = Duration::from_secs(5);

    loop {
        match connect_and_subscribe(&config.socket_path, &config.topics, &config.on_initial_state) {
            Ok(client) => {
                backoff = Duration::from_millis(100);
                info!("event stream: connected and subscribed");
                (config.on_connection_change)(true);

                // Clone the stream for reading. Keep `_client` alive so the
                // socket isn't closed — we may need the write half later.
                let _client = client;
                match _client.stream().try_clone() {
                    Err(e) => {
                        error!("event stream: failed to clone stream: {e}");
                        (config.on_connection_change)(false);
                    }
                    Ok(read_stream) => {
                        // Clear the 10s timeout inherited from IpcClient::connect —
                        // the event stream blocks indefinitely waiting for daemon events.
                        read_stream.set_read_timeout(None).ok();
                        let mut read_stream = read_stream;
                        // Read events until disconnect
                        loop {
                            match read_framed::<Response>(&mut read_stream) {
                                Some(Ok(Response::Event(event))) => {
                                    (config.on_event)(event);
                                }
                                Some(Ok(other)) => {
                                    warn!("event stream: unexpected response: {other:?}");
                                }
                                Some(Err(e)) => {
                                    error!("event stream: read error: {e}");
                                    break;
                                }
                                None => {
                                    // EOF or timeout — connection lost
                                    break;
                                }
                            }
                        }

                        info!("event stream: disconnected");
                        (config.on_connection_change)(false);
                    }
                }
            }
            Err(e) => {
                warn!("event stream: connect failed: {e}");
                (config.on_connection_change)(false);
            }
        }

        thread::sleep(backoff);
        backoff = (backoff * 2).min(max_backoff);
    }
}

fn connect_and_subscribe(path: &Path, topics: &[Topic], on_initial_state: &dyn Fn(Response)) -> io::Result<IpcClient> {
    let mut client = IpcClient::connect(path)?;

    // Version handshake — warn on mismatch but don't abort (the daemon may
    // still understand the subset of messages this client sends).
    match client.call(&Request::Hello {
        protocol_version: PROTOCOL_VERSION,
    }) {
        Ok(Response::Hello { .. }) => {}
        Ok(Response::Error(msg)) => {
            warn!("event stream: {msg}");
        }
        Ok(other) => {
            warn!("event stream: unexpected hello response: {other:?}");
        }
        Err(e) => {
            warn!("event stream: hello failed: {e}");
        }
    }

    let response = client.call(&Request::Subscribe {
        topics: topics.to_vec(),
    })?;
    match response {
        Response::Ok => {
            // Fetch full state on every (re)connect
            if let Ok(status) = client.call(&Request::Status) {
                on_initial_state(status);
            }
            Ok(client)
        }
        Response::Error(e) => Err(io::Error::other(e)),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected subscribe response: {other:?}"),
        )),
    }
}
