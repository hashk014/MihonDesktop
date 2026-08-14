//! Visual style.
//!
//! A Material-ish dark/light palette built from the accent colour of the theme
//! the user picked, applied to egui's `Visuals` once per frame.

use egui::{Color32, CornerRadius, Margin, Stroke, Vec2};

use crate::prefs::{Preferences, ThemeMode};

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Window background.
    pub background: Color32,
    /// Cards and panels sitting on the background.
    pub surface: Color32,
    /// One step brighter: hovered cards, inputs.
    pub surface_alt: Color32,
    /// Highest elevation: menus, dialogs.
    pub elevated: Color32,
    pub outline: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_dim: Color32,
    pub on_accent: Color32,
    pub error: Color32,
    pub success: Color32,
    pub is_dark: bool,
}

impl Palette {
    pub fn from_prefs(prefs: &Preferences) -> Self {
        let ([ar, ag, ab], [dr, dg, db]) = prefs.app_theme.accent();
        let accent = Color32::from_rgb(ar, ag, ab);
        let accent_dim = Color32::from_rgb(dr, dg, db);

        match prefs.theme_mode {
            ThemeMode::Dark => Self {
                background: Color32::from_rgb(0x0f, 0x10, 0x14),
                surface: Color32::from_rgb(0x17, 0x19, 0x1f),
                surface_alt: Color32::from_rgb(0x1f, 0x22, 0x2a),
                elevated: Color32::from_rgb(0x26, 0x2a, 0x34),
                outline: Color32::from_rgb(0x33, 0x38, 0x44),
                text: Color32::from_rgb(0xe8, 0xea, 0xf0),
                text_dim: Color32::from_rgb(0x99, 0xa0, 0xb0),
                accent,
                accent_dim,
                on_accent: Color32::from_rgb(0x10, 0x10, 0x16),
                error: Color32::from_rgb(0xf2, 0x6d, 0x6d),
                success: Color32::from_rgb(0x5d, 0xd3, 0x9e),
                is_dark: true,
            },
            ThemeMode::Light => Self {
                background: Color32::from_rgb(0xf7, 0xf8, 0xfb),
                surface: Color32::from_rgb(0xff, 0xff, 0xff),
                surface_alt: Color32::from_rgb(0xee, 0xf0, 0xf5),
                elevated: Color32::from_rgb(0xff, 0xff, 0xff),
                outline: Color32::from_rgb(0xd7, 0xdb, 0xe4),
                text: Color32::from_rgb(0x18, 0x1b, 0x22),
                text_dim: Color32::from_rgb(0x5f, 0x66, 0x76),
                accent: accent_dim,
                accent_dim: accent,
                on_accent: Color32::WHITE,
                error: Color32::from_rgb(0xc2, 0x39, 0x39),
                success: Color32::from_rgb(0x1f, 0x8b, 0x5f),
                is_dark: false,
            },
        }
    }

    /// Colour used behind reader pages.
    pub fn reader_background(&self, setting: crate::model::ReaderBackground) -> Color32 {
        use crate::model::ReaderBackground as Bg;
        match setting {
            Bg::White => Color32::WHITE,
            Bg::Gray => Color32::from_gray(0x40),
            Bg::Black => Color32::BLACK,
            Bg::FollowTheme => {
                if self.is_dark {
                    Color32::from_rgb(0x08, 0x08, 0x0b)
                } else {
                    Color32::from_gray(0xf0)
                }
            }
        }
    }

    /// Text colour that stays readable on top of `background`.
    pub fn contrasting(&self, background: Color32) -> Color32 {
        let luminance = 0.299 * background.r() as f32
            + 0.587 * background.g() as f32
            + 0.114 * background.b() as f32;
        if luminance > 140.0 {
            Color32::from_rgb(0x10, 0x10, 0x14)
        } else {
            Color32::from_rgb(0xf2, 0xf3, 0xf7)
        }
    }
}

pub const RADIUS_SMALL: u8 = 6;
pub const RADIUS: u8 = 10;
pub const RADIUS_LARGE: u8 = 16;

/// Installs the palette into egui's style. Called at the top of every frame so
/// theme changes take effect immediately.
pub fn apply(ctx: &egui::Context, prefs: &Preferences) -> Palette {
    let palette = Palette::from_prefs(prefs);
    // egui 0.36 keeps one style per theme; the app drives its own palette, so
    // both are written with the same values.
    ctx.all_styles_mut(|style| style_from(style, &palette));
    ctx.set_zoom_factor(prefs.ui_scale.clamp(0.7, 2.0));
    palette
}

fn style_from(style: &mut egui::Style, palette: &Palette) {
    let visuals = &mut style.visuals;
    visuals.dark_mode = palette.is_dark;
    visuals.override_text_color = Some(palette.text);
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.elevated;
    visuals.extreme_bg_color = if palette.is_dark {
        Color32::from_rgb(0x0a, 0x0b, 0x0e)
    } else {
        Color32::from_rgb(0xe9, 0xec, 0xf2)
    };
    visuals.faint_bg_color = palette.surface_alt;
    visuals.window_stroke = Stroke::new(1.0, palette.outline);
    visuals.window_corner_radius = CornerRadius::same(RADIUS_LARGE);
    visuals.menu_corner_radius = CornerRadius::same(RADIUS);
    visuals.selection.bg_fill = palette.accent_dim.gamma_multiply(0.45);
    visuals.selection.stroke = Stroke::new(1.0, palette.accent);
    visuals.hyperlink_color = palette.accent;
    visuals.error_fg_color = palette.error;
    visuals.warn_fg_color = palette.accent;

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = palette.surface;
    widgets.noninteractive.weak_bg_fill = palette.surface;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.outline);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text_dim);
    widgets.noninteractive.corner_radius = CornerRadius::same(RADIUS);

    widgets.inactive.bg_fill = palette.surface_alt;
    widgets.inactive.weak_bg_fill = palette.surface_alt;
    widgets.inactive.bg_stroke = Stroke::NONE;
    widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.inactive.corner_radius = CornerRadius::same(RADIUS);

    widgets.hovered.bg_fill = palette.elevated;
    widgets.hovered.weak_bg_fill = palette.elevated;
    widgets.hovered.bg_stroke = Stroke::new(1.0, palette.accent.gamma_multiply(0.6));
    widgets.hovered.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.hovered.corner_radius = CornerRadius::same(RADIUS);

    widgets.active.bg_fill = palette.accent.gamma_multiply(0.85);
    widgets.active.weak_bg_fill = palette.accent.gamma_multiply(0.85);
    widgets.active.bg_stroke = Stroke::new(1.0, palette.accent);
    // egui derives `strong()` text from this stroke, so it has to stay legible
    // against the panel background, not just against the accent fill.
    widgets.active.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.active.corner_radius = CornerRadius::same(RADIUS);

    widgets.open.bg_fill = palette.elevated;
    widgets.open.weak_bg_fill = palette.elevated;
    widgets.open.bg_stroke = Stroke::new(1.0, palette.outline);
    widgets.open.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.open.corner_radius = CornerRadius::same(RADIUS);

    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 7.0);
    style.spacing.menu_margin = Margin::same(6);
    style.spacing.window_margin = Margin::same(16);
    style.spacing.indent = 20.0;
    style.spacing.scroll.bar_width = 10.0;
    style.spacing.scroll.floating = false;

    style.visuals.striped = false;
    style.interaction.selectable_labels = false;

    // Typography: one coherent scale rather than egui's defaults.
    use egui::{FontFamily::Proportional, FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Heading, FontId::new(21.0, Proportional)),
        (TextStyle::Body, FontId::new(14.0, Proportional)),
        (TextStyle::Button, FontId::new(14.0, Proportional)),
        (TextStyle::Small, FontId::new(11.5, Proportional)),
        (
            TextStyle::Monospace,
            FontId::new(13.0, egui::FontFamily::Monospace),
        ),
    ]
    .into();
}

/// A card container used for list rows and grid tiles.
pub fn card(palette: &Palette) -> egui::Frame {
    egui::Frame::NONE
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::same(10))
}

/// A flat container with no fill, only padding.
pub fn plain(margin: i8) -> egui::Frame {
    egui::Frame::NONE.inner_margin(Margin::same(margin))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::AppTheme;

    #[test]
    fn dark_and_light_differ_and_stay_readable() {
        let mut prefs = Preferences {
            theme_mode: ThemeMode::Dark,
            ..Default::default()
        };
        let dark = Palette::from_prefs(&prefs);
        prefs.theme_mode = ThemeMode::Light;
        let light = Palette::from_prefs(&prefs);

        assert!(dark.is_dark && !light.is_dark);
        assert_ne!(dark.background, light.background);
        // Text on a light background must be dark, and vice versa.
        assert_eq!(dark.contrasting(Color32::WHITE).r(), 0x10);
        assert_eq!(light.contrasting(Color32::BLACK).r(), 0xf2);
    }

    #[test]
    fn every_theme_yields_a_distinct_accent() {
        let mut seen = std::collections::HashSet::new();
        for theme in AppTheme::ALL {
            let prefs = Preferences {
                app_theme: theme,
                ..Default::default()
            };
            let palette = Palette::from_prefs(&prefs);
            assert!(seen.insert(palette.accent.to_array()), "duplicate accent");
        }
    }
}
