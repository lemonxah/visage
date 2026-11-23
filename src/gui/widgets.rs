use eframe::egui;
use std::collections::VecDeque;

use egui::{
    vec2, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, RichText, Sense, Shape,
    Stroke, StrokeKind, Ui, Vec2,
};

pub const GREEN: Color32 = Color32::from_rgb(74, 214, 128);
pub const RED: Color32 = Color32::from_rgb(244, 109, 109);
pub const YELLOW: Color32 = Color32::from_rgb(242, 196, 78);
pub const BLUE: Color32 = Color32::from_rgb(98, 160, 250);
pub const CYAN: Color32 = Color32::from_rgb(56, 205, 228);
pub const PINK: Color32 = Color32::from_rgb(242, 116, 178);
pub const ACCENT: Color32 = Color32::from_rgb(120, 104, 250);
pub const BACKGROUND: Color32 = Color32::from_rgb(11, 12, 17);
pub const HEADER: Color32 = Color32::from_rgb(15, 17, 24);
pub const CARD: Color32 = Color32::from_rgb(20, 22, 30);
pub const CARD_EDGE: Color32 = Color32::from_rgb(33, 37, 49);
pub const TRACK: Color32 = Color32::from_rgb(9, 10, 14);
pub const MUTED: Color32 = Color32::from_rgb(128, 136, 156);

pub fn parse_ansi(text: &str) -> (String, Option<Color32>) {
    let mut plain = String::with_capacity(text.len());
    let mut color = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            plain.push(c);
            continue;
        }
        if chars.peek() != Some(&'[') {
            continue;
        }
        chars.next();
        let mut code = String::new();
        for next in chars.by_ref() {
            if next.is_ascii_alphabetic() {
                if next == 'm' {
                    for part in code.split(';') {
                        if let Some(found) = ansi_color(part) {
                            color = Some(found);
                        }
                    }
                }
                break;
            }
            code.push(next);
        }
    }
    (plain, color)
}

fn ansi_color(code: &str) -> Option<Color32> {
    match code {
        "31" | "91" => Some(RED),
        "32" | "92" => Some(GREEN),
        "33" | "93" => Some(YELLOW),
        "34" | "94" => Some(BLUE),
        "36" | "96" => Some(CYAN),
        _ => None,
    }
}

pub fn chip(ui: &mut Ui, text: &str, color: Option<Color32>) -> Response {
    let color = color.unwrap_or(MUTED);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::monospace(11.5), color);
    let dot = 6.0;
    let padding = vec2(9.0, 4.0);
    let size = vec2(galley.size().x + dot + 6.0, galley.size().y) + 2.0 * padding;
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(255),
            color.gamma_multiply(0.11),
            Stroke::new(1.0, color.gamma_multiply(0.28)),
            StrokeKind::Inside,
        );
        painter.circle_filled(
            Pos2::new(rect.left() + padding.x + dot / 2.0, rect.center().y),
            dot / 2.0,
            color,
        );
        painter.galley(
            Pos2::new(rect.left() + padding.x + dot + 6.0, rect.top() + padding.y),
            galley,
            color,
        );
    }
    response
}

pub fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    card(ui, |ui| {
        ui.label(
            RichText::new(title)
                .strong()
                .size(15.0)
                .color(ui.visuals().strong_text_color()),
        );
        ui.add_space(8.0);
        add(ui);
    });
}

pub fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, CARD_EDGE))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(egui::Margin::same(16))
        .shadow(egui::Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(70),
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

pub fn pin_toggle(ui: &mut Ui, pinned: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(22.0, 18.0), Sense::click());
    if response.clicked() {
        *pinned = !*pinned;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let center = rect.center();
        let hovered = response.hovered();
        if *pinned {
            painter.circle_filled(center, 6.0, YELLOW);
            painter.circle_filled(center, 2.2, CARD);
        } else {
            let color = if hovered {
                YELLOW
            } else {
                MUTED.gamma_multiply(0.6)
            };
            painter.circle_stroke(center, 5.5, Stroke::new(1.5, color));
        }
    }
    response.on_hover_text(if *pinned {
        "Pinned: drag the bar to set a value, click to release"
    } else {
        "Pin this shape to a fixed value"
    })
}

pub fn meter_edit(
    ui: &mut Ui,
    width: f32,
    value: &mut f32,
    range: (f32, f32),
    color: Color32,
) -> Response {
    let height = ui.spacing().interact_size.y * 0.8;
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    let (lo, hi) = range;
    if let Some(pointer) = response.interact_pointer_pos() {
        let t = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let next = lo + t * (hi - lo);
        if (next - *value).abs() > f32::EPSILON {
            *value = next;
            response.mark_changed();
        }
    }
    if response.double_clicked() {
        *value = 0.0f32.clamp(lo, hi);
        response.mark_changed();
    }
    paint_meter(ui, rect, *value, range, None, color, true);
    response
}

pub fn icon_button<'a>(
    ui: &Ui,
    texture: &egui::TextureHandle,
    text: impl Into<egui::WidgetText>,
) -> egui::Button<'a> {
    let tint = ui.visuals().widgets.inactive.fg_stroke.color;
    egui::Button::new((super::assets::glyph(texture, 16.0, tint), text.into()))
}

pub fn primary_button<'a>(texture: &egui::TextureHandle, text: &str) -> egui::Button<'a> {
    egui::Button::new((
        super::assets::glyph(texture, 20.0, Color32::WHITE),
        RichText::new(text)
            .size(15.0)
            .strong()
            .color(Color32::WHITE),
    ))
    .fill(ACCENT)
    .stroke(Stroke::NONE)
    .corner_radius(CornerRadius::same(10))
    .min_size(vec2(0.0, 36.0))
}

pub fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = vec2(8.0, 7.0);
        style.spacing.button_padding = vec2(12.0, 6.0);
        style.spacing.menu_margin = egui::Margin::same(8);
        style.spacing.interact_size.y = 24.0;
        style.spacing.slider_rail_height = 6.0;
        style.spacing.scroll = egui::style::ScrollStyle::floating();
        use egui::{FontFamily, TextStyle};
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(20.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(11.5, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(12.5, FontFamily::Monospace),
            ),
        ]
        .into();
    });
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = CARD;
    visuals.window_stroke = Stroke::new(1.0, CARD_EDGE);
    visuals.window_shadow = egui::Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    visuals.popup_shadow = visuals.window_shadow;
    visuals.extreme_bg_color = TRACK;
    visuals.faint_bg_color = Color32::from_rgb(25, 28, 37);
    visuals.code_bg_color = TRACK;
    visuals.hyperlink_color = CYAN;
    visuals.window_corner_radius = CornerRadius::same(14);
    visuals.menu_corner_radius = CornerRadius::same(12);
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.85);
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.slider_trailing_fill = true;
    visuals.handle_shape = egui::style::HandleShape::Circle;
    let widgets = &mut visuals.widgets;
    for state in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        state.corner_radius = CornerRadius::same(9);
    }
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, CARD_EDGE);
    widgets.noninteractive.weak_bg_fill = CARD;
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(196, 202, 216));
    widgets.inactive.bg_fill = Color32::from_rgb(33, 37, 49);
    widgets.inactive.weak_bg_fill = Color32::from_rgb(30, 34, 45);
    widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(44, 49, 64));
    widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(214, 220, 232));
    widgets.hovered.bg_fill = Color32::from_rgb(42, 47, 62);
    widgets.hovered.weak_bg_fill = Color32::from_rgb(40, 45, 60);
    widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.7));
    widgets.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    widgets.hovered.expansion = 0.0;
    widgets.active.bg_fill = Color32::from_rgb(52, 56, 84);
    widgets.active.weak_bg_fill = Color32::from_rgb(52, 56, 84);
    widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    widgets.active.expansion = 0.0;
    widgets.open.weak_bg_fill = Color32::from_rgb(40, 45, 60);
    ctx.set_visuals_of(egui::Theme::Dark, visuals);
}

pub fn meter(
    ui: &mut Ui,
    width: f32,
    value: f32,
    range: (f32, f32),
    marker: Option<f32>,
    color: Color32,
) -> Response {
    let height = ui.spacing().interact_size.y * 0.8;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    paint_meter(ui, rect, value, range, marker, color, false);
    response
}

fn paint_meter(
    ui: &Ui,
    rect: Rect,
    value: f32,
    range: (f32, f32),
    marker: Option<f32>,
    color: Color32,
    editable: bool,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let value = if value.is_finite() { value } else { 0.0 };
    let painter = ui.painter_at(rect.expand(2.0));
    let visuals = ui.visuals();
    let (lo, hi) = range;
    let to_x = |v: f32| rect.left() + ((v - lo) / (hi - lo)).clamp(0.0, 1.0) * rect.width();
    let zero = to_x(0.0f32.clamp(lo, hi));
    let x = to_x(value);
    let radius = CornerRadius::same((rect.height() / 2.0) as u8);

    painter.rect_filled(rect, radius, TRACK);
    let fill = Rect::from_x_y_ranges(zero.min(x)..=zero.max(x), rect.y_range());
    if fill.width() > 0.5 {
        painter.rect_filled(fill, radius, color.gamma_multiply(0.82));
    }
    if lo < 0.0 {
        painter.vline(
            zero,
            rect.y_range(),
            Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.5)),
        );
    }
    if let Some(marker) = marker {
        painter.vline(
            to_x(marker),
            rect.y_range(),
            Stroke::new(2.0, Color32::WHITE),
        );
    }
    if editable {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, color.gamma_multiply(0.6)),
            StrokeKind::Inside,
        );
        painter.circle_filled(
            Pos2::new(x, rect.center().y),
            rect.height() * 0.42,
            Color32::WHITE,
        );
    }
    painter.text(
        rect.right_center() - vec2(7.0, 0.0),
        Align2::RIGHT_CENTER,
        if lo < 0.0 {
            format!("{value:+.2}")
        } else {
            format!("{value:.2}")
        },
        FontId::monospace(10.5),
        visuals.strong_text_color(),
    );
}

pub fn xy_pad(
    ui: &mut Ui,
    size: f32,
    value: &mut Vec2,
    editable: bool,
    color: Color32,
) -> Response {
    let sense = if editable {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, mut response) = ui.allocate_exact_size(Vec2::splat(size), sense);
    let half = rect.width() / 2.0;

    if editable {
        if response.double_clicked() {
            *value = Vec2::ZERO;
            response.mark_changed();
        } else if let Some(pointer) = response.interact_pointer_pos() {
            let relative = (pointer - rect.center()) / half;
            let next = vec2(relative.x.clamp(-1.0, 1.0), (-relative.y).clamp(-1.0, 1.0));
            if next != *value {
                *value = next;
                response.mark_changed();
            }
        }
    }

    let painter = ui.painter_at(rect.expand(1.0));
    let visuals = ui.visuals();
    let weak = visuals.weak_text_color();
    painter.rect_filled(rect, 6.0, visuals.extreme_bg_color);
    painter.rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
        StrokeKind::Inside,
    );
    painter.hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, weak.gamma_multiply(0.4)),
    );
    painter.vline(
        rect.center().x,
        rect.y_range(),
        Stroke::new(1.0, weak.gamma_multiply(0.4)),
    );
    painter.circle_stroke(
        rect.center(),
        half * 0.5,
        Stroke::new(1.0, weak.gamma_multiply(0.25)),
    );
    let font = FontId::proportional(10.0);
    painter.text(
        rect.center_top() + vec2(0.0, 3.0),
        Align2::CENTER_TOP,
        "up",
        font.clone(),
        weak,
    );
    painter.text(
        rect.center_bottom() - vec2(0.0, 3.0),
        Align2::CENTER_BOTTOM,
        "down",
        font.clone(),
        weak,
    );
    painter.text(
        rect.left_center() + vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        "L",
        font.clone(),
        weak,
    );
    painter.text(
        rect.right_center() - vec2(4.0, 0.0),
        Align2::RIGHT_CENTER,
        "R",
        font,
        weak,
    );

    let dot = rect.center() + vec2(value.x, -value.y) * (half - 6.0);
    painter.line_segment(
        [rect.center(), dot],
        Stroke::new(1.5, color.gamma_multiply(0.6)),
    );
    painter.circle_filled(dot, 6.0, color);
    response
}

pub struct Trace<'a> {
    pub values: &'a dyn Fn(usize) -> f32,
    pub color: Color32,
    pub width: f32,
    pub dashed: bool,
}

pub fn history_plot(
    ui: &mut Ui,
    height: f32,
    times: &VecDeque<f64>,
    span: f64,
    highlight: &dyn Fn(usize) -> bool,
    traces: &[Trace],
) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 4.0, visuals.extreme_bg_color);
    for level in [0.25f32, 0.5, 0.75] {
        let y = rect.bottom() - level * rect.height();
        painter.hline(
            rect.x_range(),
            y,
            Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.15)),
        );
    }
    let Some(&last) = times.back() else {
        return response;
    };
    let to_pos = |index: usize, value: f32| {
        let x = rect.right() - ((last - times[index]) / span) as f32 * rect.width();
        Pos2::new(x, rect.bottom() - value.clamp(0.0, 1.0) * rect.height())
    };

    let mut start: Option<usize> = None;
    for index in 0..=times.len() {
        let lit = index < times.len() && highlight(index);
        match (lit, start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                let left = to_pos(from, 0.0).x;
                let right = to_pos(index - 1, 0.0).x.max(left + 1.0);
                painter.rect_filled(
                    Rect::from_x_y_ranges(left..=right, rect.y_range()),
                    0.0,
                    PINK.gamma_multiply(0.14),
                );
                start = None;
            }
            _ => {}
        }
    }

    for trace in traces {
        let points: Vec<Pos2> = (0..times.len())
            .map(|index| to_pos(index, (trace.values)(index)))
            .collect();
        let stroke = Stroke::new(trace.width, trace.color);
        if trace.dashed {
            painter.extend(Shape::dashed_line(&points, stroke, 6.0, 4.0));
        } else {
            painter.add(Shape::line(points, stroke));
        }
    }
    response
}

pub fn legend(ui: &mut Ui, items: &[(&str, Color32)]) {
    ui.horizontal(|ui| {
        for (name, color) in items {
            let (rect, _) = ui.allocate_exact_size(vec2(14.0, 4.0), Sense::hover());
            ui.painter().rect_filled(rect, 1.0, *color);
            ui.label(RichText::new(*name).small());
            ui.add_space(6.0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_colors_are_stripped_and_reported() {
        assert_eq!(
            parse_ansi("\x1b[32mFACE\x1b[0m"),
            ("FACE".into(), Some(GREEN))
        );
        assert_eq!(
            parse_ansi("\x1b[1;31mGAZE\x1b[0m"),
            ("GAZE".into(), Some(RED))
        );
        assert_eq!(parse_ansi("TICK:90/s"), ("TICK:90/s".into(), None));
    }
}
