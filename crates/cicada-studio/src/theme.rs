//! Aspecto de CICADA Studio: oscuro, con el rojo de las visuales y la misma
//! tipografía que el código del vídeo.

use std::sync::Arc;

use cicada_visual::palette::{self, Rgb};
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily};

pub const ACCENT: Color32 = Color32::from_rgb(255, 46, 36);
pub const DIM: Color32 = Color32::from_rgb(140, 140, 148);
pub const TEXT: Color32 = Color32::from_rgb(237, 232, 228);
/// Los números de línea van apagados para que no compitan con el código.
pub const GUTTER: Color32 = Color32::from_rgb(88, 84, 88);
pub const OK: Color32 = Color32::from_rgb(120, 210, 140);
pub const BG: Color32 = Color32::from_rgb(14, 12, 13);
pub const EDITOR_BG: Color32 = Color32::from_rgb(9, 8, 9);

pub fn color(rgb: Rgb) -> Color32 {
    let [r, g, b] = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    Color32::from_rgb(r, g, b)
}

pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn apply(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "mono".into(),
        Arc::new(FontData::from_static(palette::REGULAR)),
    );
    fonts.font_data.insert(
        "mono-bold".into(),
        Arc::new(FontData::from_static(palette::BOLD)),
    );
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "mono".into());
    let mut bold = vec!["mono-bold".to_string()];
    bold.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts.families.insert(self::bold(), bold);
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = Color32::from_rgb(22, 19, 20);
    v.extreme_bg_color = EDITOR_BG;
    v.faint_bg_color = Color32::from_rgb(20, 18, 19);
    v.selection.bg_fill = Color32::from_rgb(110, 24, 20);
    v.selection.stroke.color = ACCENT;
    v.hyperlink_color = ACCENT;
    v.widgets.hovered.bg_stroke.color = Color32::from_rgb(150, 40, 34);
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
    });
}
