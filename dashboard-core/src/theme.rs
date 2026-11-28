//! Shared visual identity: an ink-blue departure-board palette and the two
//! vendored typefaces (`Space Mono` for numerals, `Inter` for prose), so
//! every panel in `render.rs` draws from the same tokens instead of ad hoc
//! macroquad color constants.
use crate::board_config::TransitMode;
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
    pub const SURFACE: Color = Color {
        r: 0.1255,
        g: 0.1686,
        b: 0.2431,
        a: 1.0,
    };
    pub const CARD_ALT: Color = Color {
        r: 0.0745,
        g: 0.1059,
        b: 0.1608,
        a: 1.0,
    };
    pub const ACCENT_CYAN: Color = Color {
        r: 0.2196,
        g: 0.7412,
        b: 0.9725,
        a: 1.0,
    };
    pub const OK: Color = Color {
        r: 0.2039,
        g: 0.8275,
        b: 0.6000,
        a: 1.0,
    };
    pub const URGENT: Color = Color {
        r: 0.9843,
        g: 0.5725,
        b: 0.2353,
        a: 1.0,
    };
    pub const ALERT: Color = Color {
        r: 0.9725,
        g: 0.4431,
        b: 0.4431,
        a: 1.0,
    };
    pub const GOLD: Color = Color {
        r: 0.9804,
        g: 0.8000,
        b: 0.0824,
        a: 1.0,
    };
    pub const MODE_METRO: Color = Color {
        r: 0.0000,
        g: 0.5373,
        b: 0.7922,
        a: 1.0,
    };
    pub const MODE_TRAIN: Color = Color {
        r: 0.9255,
        g: 0.3804,
        b: 0.6235,
        a: 1.0,
    };
    pub const MODE_TRAM: Color = Color {
        r: 0.8980,
        g: 0.5373,
        b: 0.1294,
        a: 1.0,
    };
    pub const MODE_BUS: Color = Color {
        r: 0.8510,
        g: 0.1804,
        b: 0.1804,
        a: 1.0,
    };
    pub const MODE_FERRY: Color = Color {
        r: 0.0784,
        g: 0.6314,
        b: 0.6196,
        a: 1.0,
    };
}

pub fn mode_badge_color(mode: TransitMode) -> Color {
    match mode {
        TransitMode::Metro => palette::MODE_METRO,
        TransitMode::Train => palette::MODE_TRAIN,
        TransitMode::Tram => palette::MODE_TRAM,
        TransitMode::Bus => palette::MODE_BUS,
        TransitMode::Ferry | TransitMode::Ship => palette::MODE_FERRY,
    }
}

pub fn mode_label(mode: TransitMode) -> &'static str {
    match mode {
        TransitMode::Metro => "METRO",
        TransitMode::Train => "TRAIN",
        TransitMode::Tram => "TRAM",
        TransitMode::Bus => "BUS",
        TransitMode::Ferry => "FERRY",
        TransitMode::Ship => "SHIP",
    }
}

pub fn status_color(is_stale: bool, is_unavailable: bool) -> Color {
    if is_unavailable {
        palette::DOWN
    } else if is_stale {
        palette::STALE
    } else {
        palette::TEXT
    }
}

pub struct Theme<'a> {
    pub mono: &'a Font,
    pub sans: &'a Font,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_color_maps_fresh_stale_and_unavailable() {
        assert_eq!(status_color(false, false), palette::TEXT);
        assert_eq!(status_color(true, false), palette::STALE);
        assert_eq!(status_color(false, true), palette::DOWN);
        assert_eq!(status_color(true, true), palette::DOWN);
    }

    #[test]
    fn mode_badge_color_maps_all_transit_modes_to_sl_palette() {
        assert_eq!(mode_badge_color(TransitMode::Metro), palette::MODE_METRO);
        assert_eq!(mode_badge_color(TransitMode::Train), palette::MODE_TRAIN);
        assert_eq!(mode_badge_color(TransitMode::Tram), palette::MODE_TRAM);
        assert_eq!(mode_badge_color(TransitMode::Bus), palette::MODE_BUS);
        assert_eq!(mode_badge_color(TransitMode::Ferry), palette::MODE_FERRY);
        assert_eq!(mode_badge_color(TransitMode::Ship), palette::MODE_FERRY);

        for token in [
            palette::SURFACE,
            palette::CARD_ALT,
            palette::ACCENT_CYAN,
            palette::OK,
            palette::URGENT,
            palette::ALERT,
            palette::GOLD,
        ] {
            assert_eq!(token.a, 1.0);
        }
        assert_ne!(palette::SURFACE, palette::PANEL);
        assert_ne!(palette::CARD_ALT, palette::INK);
    }

    #[test]
    fn mode_label_maps_all_transit_modes_to_uppercase_labels() {
        assert_eq!(mode_label(TransitMode::Metro), "METRO");
        assert_eq!(mode_label(TransitMode::Train), "TRAIN");
        assert_eq!(mode_label(TransitMode::Tram), "TRAM");
        assert_eq!(mode_label(TransitMode::Bus), "BUS");
        assert_eq!(mode_label(TransitMode::Ferry), "FERRY");
        assert_eq!(mode_label(TransitMode::Ship), "SHIP");
    }
}
