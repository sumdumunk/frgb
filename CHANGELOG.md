# Changelog

All notable changes to frgb will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

> **Note:** The SL ring-label fix below requires manual hardware verification
> (per spec Stage 4.6). Verify visually that `frgb color --inner red -g <SL>`
> lights the inner hex (and `--outer blue` lights the outer bar) before
> tagging a release.

### Fixed

- SL fan `--inner`/`--outer` ring labels now match physical hardware.
  Previously `--inner` lit the outer bars and `--outer` lit the inner hex
  on SL fans only; this was opposite of the user's mental model and
  inconsistent with TL fans.
  **Saved profiles using `--inner`/`--outer` on SL groups will now light
  the opposite ring**; verify and update profiles as needed.
  Affects SL Wireless, SL LCD Wireless, and SL V2 fans. SL Infinity fan
  behavior is unchanged.
- `LedLayout::for_device(SlWireless | SlLcdWireless | SlV2)` now reports
  `inner_count = 13, outer_count = 8` (previously `8, 13`) to match physical
  addressing. `frgb led --index N -g <SL>` now correctly lights physical LED N
  (previously, indices 0–7 lit positions 8–15 due to the LedLayout swap
  with positional buffer routing in cmd_led).
