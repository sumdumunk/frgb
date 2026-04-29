use frgb_model::device::DeviceType;

// ---------------------------------------------------------------------------
// LedLayout
// ---------------------------------------------------------------------------
//
// Describes the physical LED ring structure for a given device type.
// LEDs per fan are indexed as: [inner_0..inner_N, outer_0..outer_M]

#[derive(Clone, Copy, Debug)]
pub struct LedLayout {
    pub inner_count: u8,
    pub outer_count: u8,
    pub total_per_fan: u8,
}

impl LedLayout {
    pub fn for_device(device_type: DeviceType) -> Self {
        match device_type {
            DeviceType::ClWireless => Self {
                inner_count: 8,
                outer_count: 16,
                total_per_fan: 24,
            },
            DeviceType::SlWireless | DeviceType::SlLcdWireless | DeviceType::SlV2 => Self {
                inner_count: 13,
                outer_count: 8,
                total_per_fan: 21,
            },
            DeviceType::TlWireless | DeviceType::TlLcdWireless => Self {
                inner_count: 8,
                outer_count: 18,
                total_per_fan: 26,
            },
            DeviceType::SlInfWireless => Self {
                inner_count: 20,
                outer_count: 38,
                total_per_fan: 58,
            },
            DeviceType::Led88 | DeviceType::V150 => Self {
                inner_count: 0,
                outer_count: 88,
                total_per_fan: 88,
            },
            DeviceType::SideArgbKit => Self {
                inner_count: 0,
                outer_count: 10,
                total_per_fan: 10,
            },
            DeviceType::HydroShiftII | DeviceType::WaterBlock | DeviceType::WaterBlock2 => Self {
                inner_count: 8,
                outer_count: 16,
                total_per_fan: 24,
            },
            _ => Self {
                inner_count: 0,
                outer_count: 0,
                total_per_fan: 0,
            },
        }
    }

    pub fn total_leds(&self, fan_count: u8) -> usize {
        self.total_per_fan as usize * fan_count as usize
    }

    pub fn inner_range(&self, fan_idx: u8) -> std::ops::Range<usize> {
        let base = fan_idx as usize * self.total_per_fan as usize;
        base..base + self.inner_count as usize
    }

    pub fn outer_range(&self, fan_idx: u8) -> std::ops::Range<usize> {
        let base = fan_idx as usize * self.total_per_fan as usize + self.inner_count as usize;
        base..base + self.outer_count as usize
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cl_layout() {
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(layout.inner_count, 8);
        assert_eq!(layout.outer_count, 16);
        assert_eq!(layout.total_per_fan, 24);
        assert_eq!(layout.total_leds(3), 72);
    }

    #[test]
    fn sl_layout() {
        let layout = LedLayout::for_device(DeviceType::SlWireless);
        assert_eq!(layout.total_per_fan, 21);
    }

    #[test]
    fn sl_layout_matches_physical_hardware() {
        use frgb_model::device::DeviceType;
        for dt in [
            DeviceType::SlWireless,
            DeviceType::SlLcdWireless,
            DeviceType::SlV2,
        ] {
            let layout = LedLayout::for_device(dt);
            assert_eq!(layout.inner_count, 13, "{dt:?} inner_count");
            assert_eq!(layout.outer_count, 8, "{dt:?} outer_count");
            assert_eq!(layout.total_per_fan, 21, "{dt:?} total_per_fan");
        }
    }

    #[test]
    fn inner_outer_ranges() {
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(layout.inner_range(0), 0..8);
        assert_eq!(layout.outer_range(0), 8..24);
        assert_eq!(layout.inner_range(1), 24..32);
        assert_eq!(layout.outer_range(1), 32..48);
    }
}
