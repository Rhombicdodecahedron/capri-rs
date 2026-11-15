//! The Edgerunners look for egui: near-black panels, neon red and cyan, square corners, mono type.

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals};

pub const RED: Color32 = Color32::from_rgb(0xff, 0x00, 0x3c);
pub const RED_DIM: Color32 = Color32::from_rgb(0x7a, 0x06, 0x24);
pub const CYAN: Color32 = Color32::from_rgb(0x00, 0xf0, 0xff);
pub const CYAN_DIM: Color32 = Color32::from_rgb(0x0b, 0x5c, 0x66);
pub const NIGHT: Color32 = Color32::from_rgb(0x0a, 0x0a, 0x12);
pub const DEEP: Color32 = Color32::from_rgb(0x06, 0x06, 0x0b);
pub const PANEL: Color32 = Color32::from_rgb(0x11, 0x11, 0x1c);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xf7, 0xff);
pub const MUTED: Color32 = Color32::from_rgb(0x7d, 0x8a, 0x99);
pub const GREEN: Color32 = Color32::from_rgb(0x2b, 0xff, 0x88);

static MONO: &[u8] = include_bytes!("../../../assets/fonts/ShareTechMono-Regular.ttf");

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("share-tech-mono".into(), std::sync::Arc::new(FontData::from_static(MONO)));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(family).or_default().insert(0, "share-tech-mono".into());
    }
    ctx.set_fonts(fonts);

    let mut v = Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.panel_fill = NIGHT;
    v.window_fill = NIGHT;
    v.extreme_bg_color = DEEP;
    v.faint_bg_color = PANEL;
    v.code_bg_color = DEEP;
    v.window_corner_radius = CornerRadius::ZERO;
    v.window_stroke = Stroke::new(1.0_f32, CYAN_DIM);
    v.selection.bg_fill = RED_DIM;
    v.selection.stroke = Stroke::new(1.0_f32, RED);
    v.hyperlink_color = CYAN;
    for (w, stroke, fill) in [
        (&mut v.widgets.noninteractive, Stroke::new(1.0_f32, CYAN_DIM), NIGHT),
        (&mut v.widgets.inactive, Stroke::new(1.0_f32, CYAN_DIM), PANEL),
        (&mut v.widgets.hovered, Stroke::new(1.0_f32, CYAN), Color32::from_rgb(0x16, 0x1c, 0x2a)),
        (&mut v.widgets.active, Stroke::new(1.5_f32, RED), Color32::from_rgb(0x2a, 0x0c, 0x18)),
        (&mut v.widgets.open, Stroke::new(1.0_f32, CYAN), PANEL),
    ] {
        w.corner_radius = CornerRadius::ZERO;
        w.bg_stroke = stroke;
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.expansion = 0.0;
    }
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, CYAN);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, RED);
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 9.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 28.0;
        s.spacing.slider_width = 260.0;
        s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
        s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
        s.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
        s.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(24.0));
    });
}
