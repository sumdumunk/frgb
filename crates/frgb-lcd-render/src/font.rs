use ab_glyph::FontRef;

const FONT_DATA: &[u8] = include_bytes!("../assets/DejaVuSansMono.ttf");

/// Returns a reference to the bundled DejaVu Sans Mono font.
pub fn font() -> FontRef<'static> {
    FontRef::try_from_slice(FONT_DATA).expect("bundled font is valid")
}
