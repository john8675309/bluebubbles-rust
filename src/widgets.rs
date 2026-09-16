use crate::theme::{palette, BLUE};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};

pub fn mark(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    ui.painter()
        .rect_filled(rect.shrink(1.0), size * 0.28, BLUE);
    let bubble = Rect::from_center_size(
        rect.center() - egui::vec2(0.0, size * 0.035),
        egui::vec2(size * 0.60, size * 0.43),
    );
    ui.painter()
        .rect_filled(bubble, size * 0.12, Color32::WHITE);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            bubble.left_bottom() + egui::vec2(size * 0.09, -size * 0.08),
            bubble.left_bottom() + egui::vec2(size * 0.09, size * 0.12),
            bubble.left_bottom() + egui::vec2(size * 0.28, -size * 0.01),
        ],
        Color32::WHITE,
        Stroke::NONE,
    ));
}

fn initials(name: &str) -> String {
    let name = name.split('@').next().unwrap_or(name);
    let words: Vec<_> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    let mut initials = String::new();
    if let Some(c) = words.first().and_then(|w| w.chars().next()) {
        initials.extend(c.to_uppercase());
    }
    if words.len() > 1 {
        if let Some(c) = words.last().and_then(|w| w.chars().next()) {
            initials.extend(c.to_uppercase());
        }
    }
    if initials.is_empty() {
        initials.push('#');
    }
    initials.chars().take(2).collect()
}

pub fn avatar(ui: &egui::Ui, center: Pos2, name: &str, key: &str, size: f32) {
    let colors = [
        Color32::from_rgb(77, 115, 188),
        Color32::from_rgb(119, 98, 181),
        Color32::from_rgb(61, 140, 140),
        Color32::from_rgb(178, 110, 112),
        Color32::from_rgb(78, 134, 166),
        Color32::from_rgb(141, 117, 82),
    ];
    let hash = key
        .bytes()
        .fold(0usize, |n, b| n.wrapping_mul(31).wrapping_add(b as usize));
    ui.painter()
        .circle_filled(center, size / 2.0, colors[hash % colors.len()]);
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        initials(name),
        FontId::proportional(size * 0.34),
        Color32::WHITE,
    );
}

fn clipped_text(ui: &egui::Ui, pos: Pos2, text: &str, size: f32, color: Color32, width: f32) {
    let mut job = egui::text::LayoutJob::simple(
        text.into(),
        FontId::proportional(size),
        color,
        width.max(1.0),
    );
    job.wrap.max_rows = 1;
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos, galley, color);
}

pub fn chat_row(
    ui: &mut egui::Ui,
    title: &str,
    preview: &str,
    time: &str,
    guid: &str,
    selected: bool,
) -> egui::Response {
    let p = palette(ui.visuals().dark_mode);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 78.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            title,
        )
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    if selected || response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, 12, if selected { p.selection } else { p.elevated });
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            12,
            Stroke::new(1.0_f32, BLUE),
            egui::StrokeKind::Inside,
        );
    }
    avatar(
        ui,
        egui::pos2(rect.left() + 31.0, rect.center().y),
        title,
        guid,
        42.0,
    );
    let left = rect.left() + 64.0;
    let right = rect.right() - 12.0;
    let time_width = if time.is_empty() { 0.0 } else { 57.0 };
    clipped_text(
        ui,
        egui::pos2(left, rect.top() + 17.0),
        title,
        15.5,
        p.text,
        right - left - time_width,
    );
    ui.painter().text(
        egui::pos2(right, rect.top() + 20.0),
        Align2::RIGHT_TOP,
        time,
        FontId::proportional(10.5),
        p.muted,
    );
    clipped_text(
        ui,
        egui::pos2(left, rect.top() + 43.0),
        preview,
        12.5,
        p.muted,
        right - left,
    );
    response.on_hover_text(format!("{title}\n{preview}"))
}

pub fn day_separator(ui: &mut egui::Ui, day: &str) {
    let p = palette(ui.visuals().dark_mode);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 42.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, day));
    let galley = ui
        .painter()
        .layout_no_wrap(day.into(), FontId::proportional(11.0), p.muted);
    let pill = Rect::from_center_size(rect.center(), galley.size() + egui::vec2(24.0, 10.0));
    ui.painter().rect_filled(pill, 16, p.sidebar);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, p.muted);
}
