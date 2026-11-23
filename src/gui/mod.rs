use eframe::egui;
use std::time::Duration;

#[cfg(feature = "qpro-tongue")]
use std::sync::atomic::Ordering;

use egui::{RichText, Ui};

use crate::core::live::LIVE;

use self::{
    assets::{glyph, glyphs},
    widgets::{
        chip, icon_button, parse_ansi, ACCENT, BACKGROUND, CARD_EDGE, HEADER, MUTED, RED, YELLOW,
    },
};

mod assets;
#[cfg(feature = "cameras")]
mod cameras;
mod face;
mod face3d;
#[cfg(feature = "qpro-tongue")]
mod tongue;
mod widgets;

const REPAINT: Duration = Duration::from_millis(33);
const REPAINT_IDLE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Face,
    Mouth,
    Cameras,
    Preview,
    Log,
}

struct App {
    tab: Tab,
    face: face::FaceView,
    face3d: face3d::Face3d,
    #[cfg(feature = "cameras")]
    cameras: cameras::CamerasView,
    #[cfg(feature = "qpro-tongue")]
    tongue: tongue::TongueView,
    log_filter: String,
    log_level: log::Level,
}

pub fn run() -> anyhow::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Visage")
        .with_app_id("visage")
        .with_inner_size([1100.0, 820.0])
        .with_min_inner_size([720.0, 480.0]);
    if let Some(icon) = assets::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Visage",
        options,
        Box::new(|cc| {
            widgets::apply_theme(&cc.egui_ctx);
            assets::load(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

impl App {
    fn new() -> Self {
        Self {
            tab: Tab::Face,
            face: face::FaceView::new(),
            face3d: face3d::Face3d::new(),
            #[cfg(feature = "cameras")]
            cameras: cameras::CamerasView::new(),
            #[cfg(feature = "qpro-tongue")]
            tongue: tongue::TongueView::new(),
            log_filter: String::new(),
            log_level: log::Level::Info,
        }
    }

    fn log_ui(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.log_filter)
                    .hint_text("Filter log…")
                    .desired_width(240.0),
            );
            egui::ComboBox::from_id_salt("log-level")
                .selected_text(self.log_level.as_str())
                .show_ui(ui, |ui| {
                    for level in [
                        log::Level::Error,
                        log::Level::Warn,
                        log::Level::Info,
                        log::Level::Debug,
                        log::Level::Trace,
                    ] {
                        ui.selectable_value(&mut self.log_level, level, level.as_str());
                    }
                });
            if ui.add(icon_button(ui, &glyphs().trash, "Clear")).clicked() {
                LIVE.log.lock().unwrap_or_else(|e| e.into_inner()).clear();
            }
        });
        ui.add_space(8.0);
        let filter = self.log_filter.to_lowercase();
        let lines: Vec<_> = LIVE
            .log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|line| line.level <= self.log_level)
            .map(|line| (line.level, line.text.clone()))
            .collect();
        let lines: Vec<_> = if filter.is_empty() {
            lines
        } else {
            lines
                .into_iter()
                .filter(|(_, text)| text.to_lowercase().contains(&filter))
                .collect()
        };
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
        widgets::card(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show_rows(ui, row_height, lines.len(), |ui, range| {
                    for (level, text) in &lines[range] {
                        let color = match level {
                            log::Level::Error => RED,
                            log::Level::Warn => YELLOW,
                            log::Level::Info => ui.visuals().text_color(),
                            _ => ui.visuals().weak_text_color(),
                        };
                        ui.add(
                            egui::Label::new(
                                RichText::new(&**text).monospace().size(12.0).color(color),
                            )
                            .truncate(),
                        );
                    }
                });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let snapshot = LIVE
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        #[cfg(feature = "qpro-tongue")]
        crate::core::ext_tracking::qpro_tongue::SHARED
            .preview_wanted
            .store(
                self.tab == Tab::Mouth && self.tongue.wants_preview(),
                Ordering::Relaxed,
            );

        egui::Panel::top("top")
            .frame(
                egui::Frame::new()
                    .fill(HEADER)
                    .stroke(egui::Stroke::new(1.0, CARD_EDGE))
                    .inner_margin(egui::Margin {
                        left: 18,
                        right: 18,
                        top: 12,
                        bottom: 12,
                    }),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Image::new(egui::load::SizedTexture::new(
                            glyphs().logo.id(),
                            egui::vec2(36.0, 36.0),
                        ))
                        .fit_to_exact_size(egui::vec2(36.0, 36.0))
                        .corner_radius(10),
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(
                            RichText::new("Visage")
                                .strong()
                                .size(20.0)
                                .color(egui::Color32::WHITE),
                        );
                        ui.label(
                            RichText::new(concat!(
                                "Linux face tracking · v",
                                env!("CARGO_PKG_VERSION")
                            ))
                            .size(11.5)
                            .color(MUTED),
                        );
                    });
                    ui.add_space(18.0);
                    tabs(ui, &mut self.tab);
                });
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    if snapshot.status.is_empty() {
                        chip(ui, "starting…", None);
                    }
                    for item in snapshot.status.iter() {
                        let (text, color) = parse_ansi(item);
                        chip(ui, &text, color);
                    }
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(&ui.ctx().global_style())
                    .inner_margin(egui::Margin::same(18)),
            )
            .show(ui, |ui| match self.tab {
                Tab::Face => {
                    egui::ScrollArea::both()
                        .auto_shrink([false; 2])
                        .show(ui, |ui| self.face.ui(ui, &snapshot));
                }
                #[cfg(feature = "qpro-tongue")]
                Tab::Mouth => {
                    egui::ScrollArea::both()
                        .auto_shrink([false; 2])
                        .show(ui, |ui| self.tongue.ui(ui));
                }
                #[cfg(not(feature = "qpro-tongue"))]
                Tab::Mouth => {}
                #[cfg(feature = "cameras")]
                Tab::Cameras => {
                    egui::ScrollArea::both()
                        .auto_shrink([false; 2])
                        .show(ui, |ui| self.cameras.ui(ui));
                }
                #[cfg(not(feature = "cameras"))]
                Tab::Cameras => {}
                Tab::Preview => self.face3d.ui(ui, &snapshot.shapes, snapshot.eyes),
                Tab::Log => self.log_ui(ui),
            });

        let idle = ui.ctx().input(|i| {
            let viewport = i.viewport();
            viewport.minimized == Some(true) || viewport.focused == Some(false)
        });
        ui.ctx()
            .request_repaint_after(if idle { REPAINT_IDLE } else { REPAINT });
    }
}

fn tabs(ui: &mut Ui, tab: &mut Tab) {
    egui::Frame::new()
        .fill(BACKGROUND)
        .stroke(egui::Stroke::new(1.0, CARD_EDGE))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                tab_button(ui, tab, Tab::Face, &glyphs().face, "Face");
                #[cfg(feature = "qpro-tongue")]
                tab_button(ui, tab, Tab::Mouth, &glyphs().tongue, "Mouth");
                #[cfg(feature = "cameras")]
                tab_button(ui, tab, Tab::Cameras, &glyphs().camera, "Cameras");
                tab_button(ui, tab, Tab::Preview, &glyphs().headset, "Preview");
                tab_button(ui, tab, Tab::Log, &glyphs().log, "Log");
            });
        });
}

fn tab_button(ui: &mut Ui, tab: &mut Tab, value: Tab, texture: &egui::TextureHandle, text: &str) {
    let selected = *tab == value;
    let color = if selected {
        egui::Color32::WHITE
    } else {
        MUTED
    };
    let button = egui::Button::new((
        glyph(texture, 17.0, color),
        RichText::new(text).size(14.0).color(color),
    ))
    .corner_radius(egui::CornerRadius::same(9))
    .min_size(egui::vec2(96.0, 32.0))
    .stroke(egui::Stroke::NONE)
    .fill(if selected {
        ACCENT
    } else {
        egui::Color32::TRANSPARENT
    });
    if ui.add(button).clicked() {
        *tab = value;
    }
}
