//! Wires the ShowRunner playback control page.

use crate::bridge::BridgeHandle;
use crate::AppWindow;

pub fn wire(window: &AppWindow, bridge: &BridgeHandle) {
    // Stop show — currently stops all sequences since the IPC has no
    // "stop by name" variant (StopSequence targets groups, not names).
    {
        let bridge = bridge.clone();
        window.on_stop_show(move |_name| {
            bridge.send(frgb_ipc::Request::StopAllSequences);
        });
    }
}
