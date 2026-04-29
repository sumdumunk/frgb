use crate::device::{DevicePair, UsbDevice};
use crate::error::Result;
use frgb_model::usb_ids::{
    PID_HYDROSHIFT_CIRCLE, PID_HYDROSHIFT_SQUARE, PID_RX, PID_SL_LCD, PID_TLV2_LCD, PID_TX, VID_LCD, VID_LIANLI,
};

pub fn find_fan_controller() -> Result<DevicePair> {
    let tx = UsbDevice::open(VID_LIANLI, PID_TX)?;
    let rx = UsbDevice::open(VID_LIANLI, PID_RX)?;
    Ok(DevicePair { tx, rx })
}

pub fn find_lcd_devices() -> Vec<UsbDevice> {
    let mut devices = Vec::new();
    for pid in [PID_SL_LCD, PID_TLV2_LCD, PID_HYDROSHIFT_CIRCLE, PID_HYDROSHIFT_SQUARE] {
        if let Ok(dev) = UsbDevice::open(VID_LCD, pid) {
            devices.push(dev);
        }
    }
    devices
}

pub fn is_fan_controller_present() -> bool {
    is_usb_device_present(VID_LIANLI, PID_TX)
}

/// Check if a USB device with the given VID/PID exists on the bus.
/// Lightweight scan — does not open or claim the device.
pub fn is_usb_device_present(vid: u16, pid: u16) -> bool {
    rusb::devices()
        .map(|devs| {
            devs.iter().any(|d| {
                d.device_descriptor()
                    .map(|desc| desc.vendor_id() == vid && desc.product_id() == pid)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}
