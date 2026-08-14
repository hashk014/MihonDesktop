//! Visual style.
//!
//! Everything the interface looks like comes out of one `Palette`, rebuilt from
//! the preferences at the top of each frame so a slider drag is visible while
//! the mouse is still down. The palette is not just an accent colour: surfaces
//! are tinted towards that accent, rounding, density and text size are all
//! user-controlled, and the widget style egui hands to every control is derived
//! from the same values.

use egui::{Color32, CornerRadius, Margin, Shadow, Stroke, Vec2};

use crate::prefs::{CardStyle, Density, Preferences};

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

    /// Corner rounding, in points, as chosen by the user.
    pub radius: u8,
    pub density: Density,
    pub card_style: CardStyle,
}

impl Palette {
    /// `dark` is resolved by the caller, since "System" needs egui's context.
    pub fn build(prefs: &Preferences, dark: bool) -> Self {
        let ([ar, ag, ab], [dr, dg, db]) = prefs.accent();
        let accent = Color32::from_rgb(ar, ag, ab);
        let accent_dim = Color32::from_rgb(dr, dg, db);
        let tint = prefs.theme_tint.clamp(0.0, 1.0);

        // The neutral base. Tinting happens afterwards so both themes share one
        // set of lightness steps and only the hue changes.
        let (base, mut palette) = if dark {
            let base: [Color32; 5] = if prefs.pure_black {
                [
                    Color32::BLACK,
                    Color32::from_rgb(0x0a, 0x0a, 0x0c),
                    Color32::from_rgb(0x15, 0x15, 0x19),
                    Color32::from_rgb(0x1e, 0x1e, 0x24),
                    Color32::from_rgb(0x2c, 0x2d, 0x35),
                ]
            } else {
                [
                    Color32::from_rgb(0x0f, 0x10, 0x14),
                    Color32::from_rgb(0x17, 0x19, 0x1f),
                    Color32::from_rgb(0x1f, 0x22, 0x2a),
                    Color32::from_rgb(0x26, 0x2a, 0x34),
                    Color32::from_rgb(0x33, 0x38, 0x44),
                ]
            };
            (
                base,
                Self {
                    background: base[0],
                    surface: base[1],
                    surface_alt: base[2],
                    elevated: base[3],
                    outline: base[4],
                    text: Color32::from_rgb(0xe8, 0xea, 0xf0),
                    text_dim: Color32::from_rgb(0x99, 0xa0, 0xb0),
                    accent,
                    accent_dim,
                    on_accent: Color32::from_rgb(0x10, 0x10, 0x16),
                    error: Color32::from_rgb(0xf2, 0x6d, 0x6d),
                    success: Color32::from_rgb(0x5d, 0xd3, 0x9e),
                    is_dark: true,
                    radius: prefs.corner_radius,
                    density: prefs.density,
                    card_style: prefs.card_style,
                },
            )
        } else {
            let base = [
                Color32::from_rgb(0xf7, 0xf8, 0xfb),
                Color32::from_rgb(0xff, 0xff, 0xff),
                Color32::from_rgb(0xee, 0xf0, 0xf5),
                Color32::from_rgb(0xff, 0xff, 0xff),
                Color32::from_rgb(0xd7, 0xdb, 0xe4),
            ];
            (
                base,
                Self {
                    background: base[0],
                    surface: base[1],
                    surface_alt: base[2],
                    elevated: base[3],
                    outline: base[4],
                    text: Color32::from_rgb(0x18, 0x1b, 0x22),
                    text_dim: Color32::from_rgb(0x5f, 0x66, 0x76),
                    // The darker variant carries a light background better.
                    accent: accent_dim,
                    accent_dim: accent,
                    on_accent: Color32::WHITE,
                    error: Color32::from_rgb(0xc2, 0x39, 0x39),
                    success: Color32::from_rgb(0x1f, 0x8b, 0x5f),
                    is_dark: false,
                    radius: prefs.corner_radius,
                    density: prefs.density,
                    card_style: prefs.card_style,
                },
            )
        };

        if tint > 0.0 {
            // Elevation reads as "closer to the accent"; the outline takes the
            // most so borders stay visible once surfaces converge.
            let hue = palette.accent;
            let weights = [0.05, 0.08, 0.12, 0.15, 0.30];
            palette.background = mix(base[0], hue, weights[0] * tint);
            palette.surface = mix(base[1], hue, weights[1] * tint);
            palette.surface_alt = mix(base[2], hue, weights[2] * tint);
            palette.elevated = mix(base[3], hue, weights[3] * tint);
            palette.outline = mix(base[4], hue, weights[4] * tint);
            // Secondary text picks up a hint of it too, but only a hint: this
            // is the colour timestamps and counts are written in.
            palette.text_dim = mix(palette.text_dim, hue, 0.12 * tint);
        }

        palette
    }

    pub fn corner(&self) -> CornerRadius {
        CornerRadius::same(self.radius)
    }

    /// Chips, badges and other small fills, which look wrong at full rounding.
    pub fn corner_small(&self) -> CornerRadius {
        CornerRadius::same((self.radius as f32 * 0.6).round() as u8)
    }

    /// Dialogs and sheets.
    pub fn corner_large(&self) -> CornerRadius {
        CornerRadius::same((self.radius as f32 * 1.6).round().min(24.0) as u8)
    }

    /// Every gap and padding in the interface is a multiple of this.
    pub fn scale(&self) -> f32 {
        self.density.scale()
    }

    /// Rounds a base spacing to whole points at the current density.
    pub fn space(&self, base: f32) -> f32 {
        (base * self.scale()).round()
    }

    pub fn margin(&self, base: f32) -> i8 {
        self.space(base).clamp(0.0, 127.0) as i8
    }

    pub fn shadow(&self) -> Shadow {
        match self.card_style {
            CardStyle::Elevated => Shadow {
                offset: [0, 2],
                blur: 12,
                spread: 0,
                color: Color32::from_black_alpha(if self.is_dark { 110 } else { 38 }),
            },
            _ => Shadow::NONE,
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
                    mix(Color32::from_rgb(0x08, 0x08, 0x0b), self.background, 0.5)
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

/// Blends `amount` of `tint` into `base`, in gamma space — which is the space
/// these greys were picked in, so the steps between them stay even.
fn mix(base: Color32, tint: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    Color32::from_rgb(
        channel(base.r(), tint.r()),
        channel(base.g(), tint.g()),
        channel(base.b(), tint.b()),
    )
}

/// Installs the palette into egui's style. Called at the top of every frame so
/// theme changes take effect immediately.
pub fn apply(ctx: &egui::Context, prefs: &Preferences) -> Palette {
    let system_is_dark = ctx.system_theme().map(|theme| theme == egui::Theme::Dark);
    let palette = Palette::build(prefs, prefs.theme_mode.is_dark(system_is_dark));
    // egui 0.36 keeps one style per theme; the app drives its own palette, so
    // both are written with the same values.
    ctx.all_styles_mut(|style| style_from(style, &palette, prefs));
    ctx.set_zoom_factor(prefs.ui_scale.clamp(0.7, 2.0));
    palette
}

fn style_from(style: &mut egui::Style, palette: &Palette, prefs: &Preferences) {
    let visuals = &mut style.visuals;
    visuals.dark_mode = palette.is_dark;
    visuals.override_text_color = Some(palette.text);
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.elevated;
    visuals.extreme_bg_color = if palette.is_dark {
        mix(palette.background, Color32::BLACK, 0.45)
    } else {
        mix(palette.background, Color32::BLACK, 0.06)
    };
    visuals.faint_bg_color = palette.surface_alt;
    visuals.window_stroke = Stroke::new(1.0, palette.outline);
    visuals.window_corner_radius = palette.corner_large();
    visuals.menu_corner_radius = palette.corner();
    visuals.window_shadow = palette.shadow();
    visuals.popup_shadow = palette.shadow();
    visuals.selection.bg_fill = palette.accent_dim.gamma_multiply(0.45);
    visuals.selection.stroke = Stroke::new(1.0, palette.accent);
    visuals.hyperlink_color = palette.accent;
    visuals.error_fg_color = palette.error;
    visuals.warn_fg_color = palette.accent;

    // Outlined cards keep their border on interactive widgets too, so a button
    // does not read as a different material from the card holding it.
    let outlined = palette.card_style == CardStyle::Outlined;
    let corner = palette.corner();

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = palette.surface;
    widgets.noninteractive.weak_bg_fill = palette.surface;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.outline);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text_dim);
    widgets.noninteractive.corner_radius = corner;

    widgets.inactive.bg_fill = palette.surface_alt;
    widgets.inactive.weak_bg_fill = palette.surface_alt;
    widgets.inactive.bg_stroke = if outlined {
        Stroke::new(1.0, palette.outline)
    } else {
        Stroke::NONE
    };
    widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.inactive.corner_radius = corner;

    widgets.hovered.bg_fill = palette.elevated;
    widgets.hovered.weak_bg_fill = palette.elevated;
    widgets.hovered.bg_stroke = Stroke::new(1.0, palette.accent.gamma_multiply(0.6));
    widgets.hovered.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.hovered.corner_radius = corner;

    widgets.active.bg_fill = palette.accent.gamma_multiply(0.85);
    widgets.active.weak_bg_fill = palette.accent.gamma_multiply(0.85);
    widgets.active.bg_stroke = Stroke::new(1.0, palette.accent);
    // egui derives `strong()` text from this stroke, so it has to stay legible
    // against the panel background, not just against the accent fill.
    widgets.active.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.active.corner_radius = corner;

    widgets.open.bg_fill = palette.elevated;
    widgets.open.weak_bg_fill = palette.elevated;
    widgets.open.bg_stroke = Stroke::new(1.0, palette.outline);
    widgets.open.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.open.corner_radius = corner;

    let scale = palette.scale();
    style.spacing.item_spacing = Vec2::new((8.0 * scale).round(), (8.0 * scale).round());
    style.spacing.button_padding = Vec2::new((12.0 * scale).round(), (7.0 * scale).round());
    style.spacing.menu_margin = Margin::same(palette.margin(6.0));
    style.spacing.window_margin = Margin::same(palette.margin(16.0));
    style.spacing.indent = (20.0 * scale).round();
    style.spacing.interact_size.y = (18.0 * scale).round().max(14.0);
    style.spacing.scroll.bar_width = (10.0 * scale).round().max(6.0);
    style.spacing.scroll.floating = false;

    style.visuals.striped = false;
    style.interaction.selectable_labels = false;
    // Zero disables egui's own easing, which is what "no animations" means for
    // hover fades, collapsing headers and scrolling.
    style.animation_time = if prefs.animations { 0.12 } else { 0.0 };

    // Typography: one coherent scale rather than egui's defaults.
    use egui::{FontFamily::Proportional, FontId, TextStyle};
    let font = prefs.font_scale.clamp(0.8, 1.5);
    style.text_styles = [
        (TextStyle::Heading, FontId::new(21.0 * font, Proportional)),
        (TextStyle::Body, FontId::new(14.0 * font, Proportional)),
        (TextStyle::Button, FontId::new(14.0 * font, Proportional)),
        (TextStyle::Small, FontId::new(11.5 * font, Proportional)),
        (
            TextStyle::Monospace,
            FontId::new(13.0 * font, egui::FontFamily::Monospace),
        ),
    ]
    .into();
}

/// A card container used for list rows and grid tiles.
pub fn card(palette: &Palette) -> egui::Frame {
    let frame = egui::Frame::NONE
        .corner_radius(palette.corner())
        .inner_margin(Margin::same(palette.margin(10.0)));
    match palette.card_style {
        CardStyle::Flat => frame.fill(palette.surface),
        CardStyle::Outlined => frame
            .fill(if palette.is_dark {
                palette.background
            } else {
                palette.surface
            })
            .stroke(Stroke::new(1.0, palette.outline)),
        CardStyle::Elevated => frame.fill(palette.surface).shadow(palette.shadow()),
    }
}

/// Rounding for a small fill, taken from the live style so call sites that do
/// not carry a palette still follow the user's choice.
pub fn small_corner(ui: &egui::Ui) -> CornerRadius {
    let radius = ui.visuals().widgets.noninteractive.corner_radius.nw;
    CornerRadius::same((radius as f32 * 0.6).round() as u8)
}

/// Padding around a screen's title bar.
pub fn header_frame(palette: &Palette) -> egui::Frame {
    egui::Frame::NONE.inner_margin(Margin::same(palette.margin(14.0)))
}

/// Padding around a screen's scrolling body.
pub fn body_frame(palette: &Palette) -> egui::Frame {
    egui::Frame::NONE.inner_margin(Margin::symmetric(palette.margin(14.0), palette.margin(6.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::{AppTheme, ThemeMode};

    fn palette_of(prefs: &Preferences) -> Palette {
        Palette::build(prefs, prefs.theme_mode.is_dark(None))
    }

    #[test]
    fn dark_and_light_differ_and_stay_readable() {
        let mut prefs = Preferences {
            theme_mode: ThemeMode::Dark,
            ..Default::default()
        };
        let dark = palette_of(&prefs);
        prefs.theme_mode = ThemeMode::Light;
        let light = palette_of(&prefs);

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
            let palette = palette_of(&prefs);
            assert!(seen.insert(palette.accent.to_array()), "duplicate accent");
        }
    }

    /// The whole point of the tint is that two themes no longer look the same
    /// once you leave the accent behind.
    #[test]
    fn tinting_moves_the_surfaces_towards_the_accent() {
        let flat = Preferences {
            theme_tint: 0.0,
            ..Default::default()
        };
        let tinted = Preferences {
            theme_tint: 1.0,
            ..Default::default()
        };
        let (flat, tinted) = (palette_of(&flat), palette_of(&tinted));

        assert_ne!(flat.background, tinted.background);
        // The default accent is violet, so the blue channel must gain the most.
        let gained = |c: fn(&Palette) -> Color32| {
            let (a, b) = (c(&flat), c(&tinted));
            b.b() as i32 - a.b() as i32
        };
        assert!(gained(|p| p.surface) > 0);
        assert!(gained(|p| p.elevated) > gained(|p| p.background));
        // A tint must never invert the elevation order.
        assert!(tinted.surface.b() < tinted.elevated.b());
    }

    #[test]
    fn a_custom_accent_wins_over_the_named_theme() {
        let prefs = Preferences {
            app_theme: AppTheme::Green,
            custom_accent: Some([0xff, 0x00, 0x88]),
            ..Default::default()
        };
        let palette = palette_of(&prefs);
        assert_eq!(palette.accent, Color32::from_rgb(0xff, 0x00, 0x88));
        // The companion colour has to be darker, or light mode loses contrast.
        assert!(palette.accent_dim.r() < palette.accent.r());
    }

    #[test]
    fn pure_black_only_applies_to_the_dark_theme() {
        let prefs = Preferences {
            pure_black: true,
            theme_tint: 0.0,
            theme_mode: ThemeMode::Dark,
            ..Default::default()
        };
        assert_eq!(palette_of(&prefs).background, Color32::BLACK);

        let light = Preferences {
            theme_mode: ThemeMode::Light,
            ..prefs.clone()
        };
        assert_ne!(palette_of(&light).background, Color32::BLACK);
    }

    #[test]
    fn density_scales_every_gap_together() {
        let widths: Vec<f32> = Density::ALL
            .iter()
            .map(|&density| {
                let prefs = Preferences {
                    density,
                    ..Default::default()
                };
                let mut style = egui::Style::default();
                style_from(&mut style, &palette_of(&prefs), &prefs);
                style.spacing.item_spacing.x
            })
            .collect();
        assert!(widths[0] < widths[1] && widths[1] < widths[2], "{widths:?}");
    }

    #[test]
    fn rounding_follows_the_preference() {
        for radius in [0, 6, 18] {
            let prefs = Preferences {
                corner_radius: radius,
                ..Default::default()
            };
            let palette = palette_of(&prefs);
            assert_eq!(palette.corner().nw, radius);
            assert!(palette.corner_small().nw <= radius);
            assert!(palette.corner_large().nw >= radius);
        }
    }

    #[test]
    fn system_mode_follows_what_the_desktop_reports() {
        assert!(!ThemeMode::System.is_dark(Some(false)));
        assert!(ThemeMode::System.is_dark(Some(true)));
        // An explicit choice ignores the desktop entirely.
        assert!(ThemeMode::Dark.is_dark(Some(false)));
        assert!(!ThemeMode::Light.is_dark(Some(true)));
    }
}
