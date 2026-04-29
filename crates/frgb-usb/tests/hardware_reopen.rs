//! Hardware-opt-in smoke test for UsbDevice::reopen.
//! Run with: cargo test -p frgb-usb --test hardware_reopen -- --ignored
//! Requires a Lian Li TX dongle plugged in (VID 0x0CF2 / PID 0x7750).

use frgb_model::usb_ids::{PID_TX, VID_LIANLI};
use frgb_usb::device::UsbDevice;

#[test]
#[ignore]
fn reopen_preserves_functionality() {
    let mut dev = UsbDevice::open(VID_LIANLI, PID_TX).expect("open TX dongle");

    // Send a small benign packet to prove the handle works.
    let probe = [0u8; 64];
    dev.write(&probe).expect("initial write");

    // Reopen.
    dev.reopen().expect("reopen");

    // Send the same packet on the new handle — should succeed.
    dev.write(&probe).expect("post-reopen write");
}

#[test]
#[ignore]
fn double_reopen_is_safe() {
    let mut dev = UsbDevice::open(VID_LIANLI, PID_TX).expect("open TX dongle");
    dev.reopen().expect("first reopen");
    dev.reopen().expect("second reopen");
    let probe = [0u8; 64];
    dev.write(&probe).expect("write after double reopen");
}
