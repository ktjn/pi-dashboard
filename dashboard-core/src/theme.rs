//! Shared visual identity: an ink-blue departure-board palette and the two
//! vendored typefaces (`Space Mono` for numerals, `Inter` for prose), so
//! every panel in `render.rs` draws from the same tokens instead of ad hoc
//! macroquad color constants.
use macroquad::color::Color;
use macroquad::text::Font;

/// Vendored copy of Space Mono (OFL, see `assets/fonts/OFL-SpaceMono.txt`),
/// used for every numeral-heavy element (clock, countdowns, standings
/// figures) so they read like a split-flap departure board.
pub const MONO_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/SpaceMono-Regular.ttf");

/// Vendored copy of Inter (OFL, see `assets/fonts/OFL-Inter.txt`), used for
/// prose (destinations, labels, team names).
pub const SANS_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// Loads both vendored fonts. Must be called after `macroquad::main` has
/// started (same requirement `load_ttf_font_from_bytes` always has), so
/// each binary's `main` calls this once at startup rather than `theme.rs`
/// trying to load them lazily itself.
pub fn load_fonts() -> Result<(Font, Font), macroquad::Error> {
    let mono = macroquad::text::load_ttf_font_from_bytes(MONO_FONT_BYTES)?;
    let sans = macroquad::text::load_ttf_font_from_bytes(SANS_FONT_BYTES)?;
    Ok((mono, sans))
}

pub mod palette {
    use macroquad::color::Color;

    pub const INK: Color = Color {
        r: 0.0549,
        g: 0.0784,
        b: 0.1255,
        a: 1.0,
    };
    pub const PANEL: Color = Color {
        r: 0.0941,
        g: 0.1294,
        b: 0.1922,
        a: 1.0,
    };
    pub const RULE: Color = Color {
        r: 0.1647,
        g: 0.2118,
        b: 0.2784,
        a: 1.0,
    };
    pub const TEXT: Color = Color {
        r: 0.9059,
        g: 0.9255,
        b: 0.9490,
        a: 1.0,
    };
    pub const MUTED: Color = Color {
        r: 0.4902,
        g: 0.5490,
        b: 0.6275,
        a: 1.0,
    };
    pub const FRESH: Color = Color {
        r: 0.2392,
        g: 0.8627,
        b: 0.5922,
        a: 1.0,
    };
    pub const STALE: Color = Color {
        r: 0.9412,
        g: 0.6353,
        b: 0.2392,
        a: 1.0,
    };
    pub const DOWN: Color = Color {
        r: 0.2941,
        g: 0.3333,
        b: 0.4118,
        a: 1.0,
    };
}

/// Maps a panel's fresh/stale/unavailable state to its text color: neutral
/// for normal data, amber for stale, slate for unavailable. The teal accent
/// (`palette::FRESH`) is reserved for data itself (line numbers, points),
/// never used as the "everything is fine" text color - that would spend the
/// one accent on the common case instead of the exception.
pub fn status_color(is_stale: bool, is_unavailable: bool) -> Color {
    if is_unavailable {
        palette::DOWN
    } else if is_stale {
        palette::STALE
    } else {
        palette::TEXT
    }
}

/// Bundles the two vendored fonts so render functions take one reference
/// instead of two positional font arguments.
pub struct Theme<'a> {
    pub mono: &'a Font,
    pub sans: &'a Font,
}
