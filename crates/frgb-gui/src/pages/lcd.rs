//! Wires the LCD page callbacks and fetches LCD device list.

use std::sync::{Arc, Mutex};

use slint::ComponentHandle;

use crate::bridge::BridgeHandle;
use crate::AppWindow;

/// Shared preset list — populated by fetch_presets, read by apply handler.
pub type PresetList = Arc<Mutex<Vec<frgb_model::lcd::LcdPreset>>>;

/// Try to open a native file-picker dialog.
/// Tries zenity first, then kdialog. Returns the picked path or an empty string.
fn pick_file(filter: &str) -> slint::SharedString {
    // Try zenity
    if let Ok(output) = std::process::Command::new("zenity")
        .args(["--file-selection", "--file-filter", filter])
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return path.into();
            }
        }
    }
    // Try kdialog
    if let Ok(output) = std::process::Command::new("kdialog")
        .args(["--getopenfilename", ".", filter])
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return path.into();
            }
        }
    }
    slint::SharedString::default()
}

pub fn wire(window: &AppWindow, bridge: &BridgeHandle, presets: &PresetList) {
    let bridge = bridge.clone();

    {
        let presets = presets.clone();
        window.on_apply_lcd(
            move |lcd_index, content_type, brightness, rotation, sensor, style, color, clock_style, text, file_path| {
                let config = crate::lcd_convert::build_lcd_config(
                    &content_type,
                    brightness,
                    rotation,
                    &sensor,
                    &style,
                    &color,
                    clock_style,
                    &text,
                    &file_path,
                    &presets.lock().unwrap(),
                );
                bridge.send(frgb_ipc::Request::SetLcd {
                    lcd_index: lcd_index as u8,
                    config,
                });
            },
        );
    }

    window.on_browse_image(|| pick_file("Image Files | *.png *.jpg *.jpeg *.bmp"));

    window.on_browse_gif(|| pick_file("GIF Files | *.gif"));
}

/// Fetch preset names from daemon and populate the LCD page.
pub fn fetch_presets(window: &AppWindow, bridge: &BridgeHandle, presets: &PresetList) {
    let w = window.as_weak();
    let presets = presets.clone();
    bridge.call(frgb_ipc::Request::ListPresets, move |resp| {
        if let frgb_ipc::Response::Presets(fetched) = resp {
            let names: Vec<slint::SharedString> = fetched
                .iter()
                .map(|p| slint::SharedString::from(p.name.as_str()))
                .collect();

            // Collect raw thumbnail data for decoding on the UI thread
            let thumb_data: Vec<(String, String, i32, Vec<u8>)> = fetched
                .iter()
                .map(|p| {
                    (
                        p.name.clone(),
                        format!("{:?}", p.category),
                        p.index as i32,
                        p.thumbnail.clone(),
                    )
                })
                .collect();

            *presets.lock().unwrap() = fetched;
            slint::invoke_from_event_loop(move || {
                if let Some(window) = w.upgrade() {
                    window.set_preset_names(slint::ModelRc::new(slint::VecModel::from(names)));

                    let grid_data: Vec<crate::LcdPresetData> = thumb_data
                        .into_iter()
                        .map(|(name, category, index, thumb_bytes)| {
                            let thumb_image = if thumb_bytes.is_empty() {
                                slint::Image::default()
                            } else {
                                match image::load_from_memory(&thumb_bytes) {
                                    Ok(img) => {
                                        let rgba = img.to_rgba8();
                                        let (w, h) = (rgba.width(), rgba.height());
                                        let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                                            rgba.as_raw(),
                                            w,
                                            h,
                                        );
                                        slint::Image::from_rgba8(buf)
                                    }
                                    Err(_) => slint::Image::default(),
                                }
                            };
                            crate::LcdPresetData {
                                name: slint::SharedString::from(name.as_str()),
                                category: slint::SharedString::from(category.as_str()),
                                index,
                                thumbnail: thumb_image,
                            }
                        })
                        .collect();
                    window.set_lcd_presets(slint::ModelRc::new(slint::VecModel::from(grid_data)));
                }
            })
            .ok();
        }
    });
}

/// Fetch LCD device list from daemon and populate the LCD page.
pub fn fetch_lcd_devices(window: &AppWindow, bridge: &BridgeHandle) {
    use crate::LcdDeviceData;
    let w = window.as_weak();
    bridge.call(frgb_ipc::Request::ListLcdDevices, move |resp| {
        if let frgb_ipc::Response::LcdDevices(devices) = resp {
            let slint_devices: Vec<LcdDeviceData> = devices
                .iter()
                .map(|d| LcdDeviceData {
                    index: d.index as i32,
                    name: slint::SharedString::from(&d.name),
                    width: d.width as i32,
                    height: d.height as i32,
                })
                .collect();
            let count = slint_devices.len() as i32;
            slint::invoke_from_event_loop(move || {
                if let Some(window) = w.upgrade() {
                    window.set_lcd_devices(slint::ModelRc::new(slint::VecModel::from(slint_devices)));
                    window.set_lcd_count(count);
                }
            })
            .ok();
        }
    });
}
