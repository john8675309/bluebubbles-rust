use eframe::egui::{self, Color32, FontId, TextStyle};

pub const BLUE: Color32 = Color32::from_rgb(52, 120, 246);

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Color32,
    pub sidebar: Color32,
    pub surface: Color32,
    pub elevated: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub selection: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            background: Color32::from_rgb(17, 22, 31),
            sidebar: Color32::from_rgb(22, 28, 39),
            surface: Color32::from_rgb(26, 33, 46),
            elevated: Color32::from_rgb(34, 43, 58),
            border: Color32::from_rgb(43, 53, 70),
            text: Color32::from_rgb(233, 238, 247),
            muted: Color32::from_rgb(150, 164, 185),
            selection: Color32::from_rgb(31, 54, 87),
        }
    } else {
        Palette {
            background: Color32::from_rgb(247, 249, 253),
            sidebar: Color32::from_rgb(240, 244, 250),
            surface: Color32::WHITE,
            elevated: Color32::from_rgb(231, 237, 246),
            border: Color32::from_rgb(218, 226, 237),
            text: Color32::from_rgb(31, 43, 64),
            muted: Color32::from_rgb(99, 116, 140),
            selection: Color32::from_rgb(221, 234, 255),
        }
    }
}

pub fn panel(fill: Color32, margin: i8) -> egui::Frame {
    egui::Frame::new().fill(fill).inner_margin(margin)
}

pub fn apply(ctx: &egui::Context, dark: bool) {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let p = palette(dark);
    visuals.override_text_color = Some(p.text);
    visuals.panel_fill = p.background;
    visuals.window_fill = p.surface;
    visuals.extreme_bg_color = p.surface;
    visuals.faint_bg_color = p.sidebar;
    visuals.hyperlink_color = BLUE;
    visuals.error_fg_color = if dark {
        Color32::from_rgb(242, 135, 146)
    } else {
        Color32::from_rgb(185, 50, 65)
    };
    visuals.window_corner_radius = 16.into();
    visuals.window_stroke = egui::Stroke::new(1.0_f32, p.border);
    visuals.widgets.noninteractive.bg_fill = p.surface;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, p.border);
    visuals.widgets.noninteractive.fg_stroke.color = p.muted;
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.corner_radius = 8.into();
        widget.bg_stroke = egui::Stroke::NONE;
        widget.fg_stroke.color = p.text;
    }
    visuals.widgets.inactive.bg_fill = p.elevated;
    visuals.widgets.inactive.weak_bg_fill = p.elevated;
    visuals.widgets.hovered.bg_fill = p.selection;
    visuals.widgets.hovered.weak_bg_fill = p.selection;
    visuals.widgets.active.bg_fill = p.selection;
    visuals.selection.bg_fill = p.selection;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, BLUE);
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(16.0));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(15.0));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(12.0));
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(22.0));
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 8.0);
        style.spacing.window_margin = 20.into();
        style.spacing.interact_size.y = 32.0;
    });
}
