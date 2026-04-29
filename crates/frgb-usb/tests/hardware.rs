//! Integration tests that require real Lian Li hardware.
//! Run with: cargo test -p frgb-usb --test hardware -- --ignored --nocapture --test-threads=1

use frgb_protocol::{decode, encode};
use frgb_usb::{find_fan_controller, find_lcd_devices, is_fan_controller_present};
use std::time::Duration;

/// Drain any stale data from a device's read buffer.
fn drain(dev: &frgb_usb::UsbDevice) {
    for _ in 0..10 {
        if dev.read_timeout(Duration::from_millis(50)).is_err() {
            break;
        }
    }
}

#[test]
#[ignore]
fn detect_fan_controller() {
    assert!(is_fan_controller_present(), "No Lian Li fan controller detected on USB");
}

#[test]
#[ignore]
fn tx_sync() {
    let pair = find_fan_controller().expect("Failed to open fan controller");

    // Drain any stale data
    drain(&pair.tx);

    let sync = encode::encode_tx_sync(0x08);
    pair.tx.write(&sync).expect("Failed to send TX sync");

    let resp = pair.tx.read().expect("No TX sync response");
    let parsed = decode::decode_tx_sync(&resp).expect("Failed to parse TX sync");

    println!("TX Device ID: {}", parsed.tx_device_id);
    println!("Firmware: 0x{:04x}", parsed.firmware_version);
    println!("System clock: {} ms", parsed.system_clock_ms);
    assert_ne!(parsed.tx_device_id, frgb_model::device::DeviceId::ZERO);
}

#[test]
#[ignore]
fn query_status() {
    let pair = find_fan_controller().expect("Failed to open fan controller");

    // Drain stale data
    drain(&pair.rx);

    // Use L-Connect device query (page_count=1)
    let query = encode::encode_device_query(1);
    pair.rx.write(&query).expect("Failed to send device query");

    let mut buf = Vec::new();
    for _ in 0..7 {
        match pair.rx.read_timeout(Duration::from_millis(500)) {
            Ok(data) => buf.extend_from_slice(&data),
            Err(_) => break,
        }
    }

    assert!(!buf.is_empty(), "No device query response received");
    println!("Read {} bytes ({} packets)", buf.len(), buf.len() / 64);

    let response = decode::decode_device_query(&buf);
    println!("Num devices: {}", response.num_devices);
    if let Some(fw) = response.rx_firmware {
        println!("RX Firmware: 0x{:04x}", fw);
    }

    for rec in &response.records {
        println!(
            "  Group {}: mac={} master={} ch={} type={} fans={}{}",
            rec.group,
            rec.mac_addr,
            rec.master_mac_addr,
            rec.channel,
            rec.dev_type,
            rec.fan_num,
            if rec.is_inf_right_attach {
                " (INF right-attach)"
            } else {
                ""
            },
        );
        println!(
            "    fans_type: [{}, {}, {}, {}]",
            rec.fans_type[0], rec.fans_type[1], rec.fans_type[2], rec.fans_type[3]
        );
        println!(
            "    RPM: [{}, {}, {}, {}]",
            rec.fans_speed[0], rec.fans_speed[1], rec.fans_speed[2], rec.fans_speed[3]
        );
        println!(
            "    PWM: [{}, {}, {}, {}]",
            rec.fans_pwm[0], rec.fans_pwm[1], rec.fans_pwm[2], rec.fans_pwm[3]
        );
    }
}

#[test]
#[ignore]
fn discover_all_devices() {
    let pair = find_fan_controller().expect("Failed to open fan controller");

    // Drain stale data from both devices
    drain(&pair.tx);
    drain(&pair.rx);

    // TX sync
    let sync = encode::encode_tx_sync(0x08);
    pair.tx.write(&sync).expect("TX sync failed");
    let tx_resp = pair.tx.read().expect("No TX sync response");
    let tx_info = decode::decode_tx_sync(&tx_resp).expect("TX sync parse failed");
    println!("TX Device ID: {}", tx_info.tx_device_id);
    println!("TX Firmware: 0x{:04x}", tx_info.firmware_version);

    // L-Connect device query
    let query = encode::encode_device_query(1);
    pair.rx.write(&query).expect("Device query failed");

    let mut buf = Vec::new();
    for _ in 0..7 {
        match pair.rx.read_timeout(Duration::from_millis(500)) {
            Ok(data) => buf.extend_from_slice(&data),
            Err(_) => break,
        }
    }

    let response = decode::decode_device_query(&buf);
    println!("Found {} device(s):", response.num_devices);
    for rec in &response.records {
        let bound = if rec.master_mac_addr == tx_info.tx_device_id {
            "OURS"
        } else if rec.dev_type == 0xFF {
            "MASTER"
        } else {
            "other"
        };
        println!(
            "  [{bound}] Group {}: mac={} type={} fans={} ch={} rpm=[{},{},{},{}]",
            rec.group,
            rec.mac_addr,
            rec.dev_type,
            rec.fan_num,
            rec.channel,
            rec.fans_speed[0],
            rec.fans_speed[1],
            rec.fans_speed[2],
            rec.fans_speed[3]
        );
    }

    // LCD devices
    let lcds = find_lcd_devices();
    println!("Found {} LCD devices", lcds.len());
}
