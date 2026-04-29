use std::sync::mpsc;
use std::thread;

use frgb_ipc::{
    run_event_loop, socket_path, Event, EventStreamConfig, IpcClient, Request, Response, Topic, PROTOCOL_VERSION,
};
use tracing::error;

/// Commands sent from the UI thread to the bridge.
pub enum BridgeCommand {
    /// Send a request and discard the response (fire-and-forget).
    Send(Request),
    /// Send a request and deliver the response via callback.
    Call(Request, Box<dyn FnOnce(Response) + Send>),
}

/// Handle for the UI thread to communicate with the IPC bridge.
#[derive(Clone)]
pub struct BridgeHandle {
    tx: mpsc::SyncSender<BridgeCommand>,
}

impl BridgeHandle {
    /// Send a fire-and-forget request to the daemon.
    pub fn send(&self, request: Request) {
        tracing::debug!("bridge.send: {:?}", std::mem::discriminant(&request));
        if let Err(e) = self.tx.send(BridgeCommand::Send(request)) {
            tracing::error!("bridge.send: channel send failed: {e}");
        }
    }

    /// Send a request and invoke `cb` with the response.
    ///
    /// The callback runs on the request thread, NOT the UI thread.
    /// Use `slint::invoke_from_event_loop` inside `cb` to touch UI state.
    pub fn call(&self, request: Request, cb: impl FnOnce(Response) + Send + 'static) {
        let _ = self.tx.send(BridgeCommand::Call(request, Box::new(cb)));
    }
}

/// Callbacks the bridge invokes to push state into the UI.
pub struct BridgeCallbacks {
    pub on_event: Box<dyn Fn(Event) + Send>,
    pub on_connected: Box<dyn Fn(bool) + Send>,
    pub on_initial_state: Box<dyn Fn(Response) + Send>,
}

/// Spawn the IPC bridge. Returns a handle for sending commands.
///
/// Starts two threads:
/// 1. Request thread — reads commands from the channel, sends them over a dedicated IpcClient.
/// 2. Event thread — subscribes to daemon events and forwards them via callbacks.
pub fn spawn(callbacks: BridgeCallbacks) -> BridgeHandle {
    // Bounded channel — backpressure if daemon is slow. 64 is generous;
    // the speed debouncer already collapses slider drag to ≤10/s.
    let (tx, rx) = mpsc::sync_channel::<BridgeCommand>(64);

    // Request thread — handles UI-initiated commands
    let path = socket_path();
    thread::Builder::new()
        .name("ipc-request".into())
        .spawn(move || {
            let mut client: Option<IpcClient> = None;

            tracing::info!("ipc-request thread started, waiting for commands");
            for cmd in rx {
                tracing::debug!("ipc-request: received command");
                // Lazy-connect (or reconnect) on first use
                if client.is_none() {
                    match IpcClient::connect(&path) {
                        Ok(mut c) => {
                            // Version handshake on new connection
                            match c.call(&Request::Hello {
                                protocol_version: PROTOCOL_VERSION,
                            }) {
                                Ok(Response::Error(msg)) => {
                                    error!("ipc handshake rejected: {msg}");
                                    if let BridgeCommand::Call(_, cb) = cmd {
                                        cb(Response::Error(format!("handshake failed: {msg}")));
                                    }
                                    continue;
                                }
                                Err(e) => {
                                    error!("ipc handshake failed: {e}");
                                    if let BridgeCommand::Call(_, cb) = cmd {
                                        cb(Response::Error(format!("handshake error: {e}")));
                                    }
                                    continue;
                                }
                                _ => {}
                            }
                            client = Some(c);
                        }
                        Err(e) => {
                            error!("ipc request: connect failed: {e}");
                            if let BridgeCommand::Call(_, cb) = cmd {
                                cb(Response::Error(format!("not connected: {e}")));
                            }
                            continue;
                        }
                    }
                }

                let c = client.as_mut().unwrap();
                match cmd {
                    BridgeCommand::Send(req) => {
                        // Use call() to read and discard the response, preventing
                        // orphan responses from desyncing subsequent Call commands.
                        match c.call(&req) {
                            Ok(_) => {}
                            Err(e) => {
                                error!("ipc request: send failed: {e}");
                                client = None;
                            }
                        }
                    }
                    BridgeCommand::Call(req, cb) => match c.call(&req) {
                        Ok(resp) => cb(resp),
                        Err(e) => {
                            error!("ipc request: call failed: {e}");
                            cb(Response::Error(format!("ipc error: {e}")));
                            client = None;
                        }
                    },
                }
            }
        })
        .expect("failed to spawn ipc-request thread");

    // Event thread — subscribes to daemon events
    thread::Builder::new()
        .name("ipc-event".into())
        .spawn(move || {
            let config = EventStreamConfig {
                socket_path: socket_path(),
                topics: vec![
                    Topic::Rpm,
                    Topic::Temperature,
                    Topic::DeviceChange,
                    Topic::Speed,
                    Topic::Rgb,
                    Topic::Profile,
                    Topic::Alert,
                    Topic::Power,
                ],
                on_event: callbacks.on_event,
                on_connection_change: callbacks.on_connected,
                on_initial_state: callbacks.on_initial_state,
            };
            run_event_loop(config);
        })
        .expect("failed to spawn ipc-event thread");

    BridgeHandle { tx }
}
