use eframe::egui;
use std::path::PathBuf;

use egui::{
    pos2, vec2, Color32, ColorImage, Pos2, Rect, RichText, Sense, Stroke, StrokeKind,
    TextureHandle, TextureOptions, Ui, Vec2,
};

use crate::core::ext_tracking::camera::{
    self, babble_face::OUTPUT_NAMES as MOUTH_NAMES, etvr_eyes::OUTPUT_NAMES as EYE_NAMES,
    onefilter::FilterParams, process::Roi, CameraConfig, CameraId, CameraTelemetry, EyesTelemetry,
    LinkState, ModelTelemetry, MouthTelemetry, Preview, EXPRESSION_NAMES, SHARED,
};

use super::{
    assets::glyphs,
    widgets::{chip, icon_button, meter, section, xy_pad, BLUE, CYAN, GREEN, PINK, RED, YELLOW},
};

const PREVIEW_WIDTH: f32 = 280.0;
const INPUT_DISPLAY: f32 = 112.0;
const MESSAGE_SECONDS: f64 = 5.0;
const NAME_WIDTH: f32 = 128.0;
const BAR_WIDTH: f32 = 120.0;

#[derive(Default)]
struct Textures {
    raw: Option<TextureHandle>,
    raw_generation: u64,
    raw_size: [usize; 2],
    input: Option<TextureHandle>,
    input_generation: u64,
}

struct Message {
    text: String,
    error: bool,
    at: f64,
}

pub struct CamerasView {
    textures: [Textures; 3],
    sources: [String; 3],
    models: [String; 2],
    synced: bool,
    drag: Option<(CameraId, Pos2, Pos2)>,
    message: Option<Message>,
    saved: Option<CameraConfig>,
}

impl Default for CamerasView {
    fn default() -> Self {
        Self::new()
    }
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn path_text(path: &Option<PathBuf>) -> String {
    path.as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

fn link_chip(ui: &mut Ui, camera: &CameraTelemetry) {
    let (text, color) = match camera.state {
        LinkState::Off => ("off".to_string(), None),
        LinkState::Connecting => ("connecting".to_string(), Some(YELLOW)),
        LinkState::Streaming => (format!("{:.0} fps", camera.fps), Some(GREEN)),
        LinkState::Retrying => ("retrying".to_string(), Some(RED)),
    };
    chip(ui, &text, color);
    if camera.state == LinkState::Streaming && camera.width > 0 {
        chip(ui, &format!("{}x{}", camera.width, camera.height), None);
    }
    if camera.bad_frames > 0 {
        chip(
            ui,
            &format!("{} bad frames", camera.bad_frames),
            Some(YELLOW),
        );
    }
}

fn model_chip(ui: &mut Ui, model: &ModelTelemetry) {
    if model.ready {
        chip(
            ui,
            &format!("model {:.0} fps · {:.1} ms", model.fps, model.latency_ms),
            Some(GREEN),
        );
    } else if model.error.is_some() {
        chip(ui, "model not running", Some(RED));
    } else if model.path.is_some() {
        chip(ui, "model loading", Some(YELLOW));
    } else {
        chip(ui, "no model", None);
    }
}

fn error_line(ui: &mut Ui, error: Option<&String>) {
    if let Some(error) = error {
        ui.label(RichText::new(error).small().color(RED));
    }
}

fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).small().weak());
}

impl CamerasView {
    pub fn new() -> Self {
        Self {
            textures: Default::default(),
            sources: Default::default(),
            models: Default::default(),
            synced: false,
            drag: None,
            message: None,
            saved: camera::saved_config(),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        camera::request_preview();
        let now = ui.input(|i| i.time);
        self.update_textures(ui);

        if !camera::running() {
            section(ui, "Direct camera tracking is not running", |ui| {
                ui.label("Start Visage with a face provider (for example visage openxr or visage babble) built with the cameras feature.");
                ui.label("Pass --babble-camera, --babble-model, --etvr-left, --etvr-right and --etvr-model, or set the sources here once Visage is running.");
            });
            return;
        }

        let config = camera::config();
        if !self.synced {
            for id in CameraId::ALL {
                self.sources[id.index()] = config.source(id).unwrap_or_default().to_string();
            }
            self.models = [
                path_text(&config.mouth.model),
                path_text(&config.eyes.model),
            ];
            self.synced = true;
        }
        let cameras = lock(&SHARED.cameras).clone();
        let mouth = lock(&SHARED.mouth).clone();
        let eyes = lock(&SHARED.eyes).clone();
        let mut edited = config.clone();

        self.header_ui(ui, now, &config);
        ui.add_space(6.0);
        section(ui, "Babble mouth camera", |ui| {
            self.mouth_ui(
                ui,
                now,
                &cameras[CameraId::Mouth.index()],
                &mouth,
                &mut edited,
            )
        });
        ui.add_space(6.0);
        section(ui, "EyeTrackVR eye cameras", |ui| {
            self.eyes_ui(ui, now, &cameras, &eyes, &mut edited)
        });

        if edited != config {
            commit(&config, &edited);
        }
    }

    fn say(&mut self, now: f64, text: String, error: bool) {
        self.message = Some(Message {
            text,
            error,
            at: now,
        });
    }

    fn report(&mut self, now: f64, result: anyhow::Result<PathBuf>) {
        match result {
            Ok(path) => {
                self.saved = camera::saved_config();
                self.say(now, format!("Saved to {}", path.display()), false);
            }
            Err(e) => self.say(now, format!("Could not save: {e:#}"), true),
        }
    }

    fn header_ui(&mut self, ui: &mut Ui, now: f64, config: &CameraConfig) {
        ui.horizontal_wrapped(|ui| {
            let dirty = match self.saved.as_ref() {
                Some(saved) => saved != config,
                None => *config != CameraConfig::default(),
            };
            if ui
                .add_enabled(dirty, icon_button(ui, &glyphs().save, "Save"))
                .on_hover_text("Write sources, ROI, rotation and calibration to cameras.json")
                .clicked()
            {
                let result = camera::save_config();
                self.report(now, result);
            }
            if ui
                .add_enabled(
                    self.saved.is_some() && dirty,
                    icon_button(ui, &glyphs().revert, "Revert"),
                )
                .on_hover_text("Go back to the last saved settings")
                .clicked()
            {
                if let Some(saved) = self.saved.clone() {
                    camera::update_config(|current| *current = saved);
                    self.synced = false;
                    self.say(now, "Reverted to the saved settings".into(), false);
                }
            }
            if dirty {
                chip(ui, "unsaved changes", Some(YELLOW));
            }
            ui.label(
                RichText::new(camera::config_path().display().to_string())
                    .small()
                    .weak(),
            );
        });
        if let Some(message) = self.message.as_ref() {
            if now - message.at < MESSAGE_SECONDS {
                ui.label(
                    RichText::new(&message.text)
                        .small()
                        .color(if message.error { RED } else { GREEN }),
                );
            } else {
                self.message = None;
            }
        }
    }

    fn source_row(
        &mut self,
        ui: &mut Ui,
        id: CameraId,
        label: &str,
        camera: &CameraTelemetry,
        edited: &mut CameraConfig,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.label(label);
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.sources[id.index()])
                    .hint_text("/dev/ttyUSB0, http://host/ or /dev/video0")
                    .desired_width(260.0),
            );
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Connect").clicked() || entered {
                edited.set_source(id, Some(self.sources[id.index()].clone()));
            }
            if ui
                .add_enabled(edited.source(id).is_some(), egui::Button::new("Stop"))
                .clicked()
            {
                edited.set_source(id, None);
                self.sources[id.index()].clear();
            }
            link_chip(ui, camera);
        });
        if camera.state == LinkState::Streaming && !camera.description.is_empty() {
            hint(ui, &camera.description);
        }
        error_line(ui, camera.error.as_ref());
    }

    fn model_row(
        &mut self,
        ui: &mut Ui,
        slot: usize,
        hint_text: &str,
        model: &ModelTelemetry,
        target: &mut Option<PathBuf>,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Model");
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.models[slot])
                    .hint_text(hint_text)
                    .desired_width(320.0),
            );
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Load").clicked() || entered {
                let text = self.models[slot].trim();
                *target = (!text.is_empty()).then(|| PathBuf::from(text));
            }
            model_chip(ui, model);
        });
        error_line(ui, model.error.as_ref());
    }

    fn preview_ui(
        &mut self,
        ui: &mut Ui,
        id: CameraId,
        camera: &CameraTelemetry,
        edited: &mut CameraConfig,
    ) {
        let textures = &self.textures[id.index()];
        let [width, height] = textures.raw_size;
        let aspect = if width > 0 && height > 0 {
            height as f32 / width as f32
        } else {
            0.75
        };
        let size = vec2(PREVIEW_WIDTH, (PREVIEW_WIDTH * aspect).clamp(80.0, 400.0));
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let streaming = camera.state == LinkState::Streaming;
        let normalized = |pos: Pos2| {
            pos2(
                ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
            )
        };

        if streaming {
            if response.double_clicked() {
                edited.view_mut(id).roi = Roi::default();
                self.drag = None;
            } else if response.drag_started() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let start = normalized(pos);
                    self.drag = Some((id, start, start));
                }
            } else if let Some((drag_id, start, _)) = self.drag {
                if drag_id == id {
                    if let Some(pos) = response.interact_pointer_pos() {
                        self.drag = Some((id, start, normalized(pos)));
                    }
                    if response.drag_stopped() || !response.dragged() {
                        if let Some((_, start, end)) = self.drag.take() {
                            if (end - start).length() > 0.02 {
                                edited.view_mut(id).roi =
                                    Roi::from_corners((start.x, start.y), (end.x, end.y));
                            }
                        }
                    }
                }
            }
        }

        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        painter.rect_filled(rect, 6.0, visuals.extreme_bg_color);
        match textures.raw.as_ref().filter(|_| streaming) {
            Some(texture) => {
                painter.image(
                    texture.id(),
                    rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                let roi = match self.drag {
                    Some((drag_id, start, end)) if drag_id == id => {
                        Roi::from_corners((start.x, start.y), (end.x, end.y))
                    }
                    _ => edited.view(id).roi,
                };
                let roi_rect = Rect::from_min_size(
                    rect.min + vec2(roi.x * rect.width(), roi.y * rect.height()),
                    vec2(roi.width * rect.width(), roi.height * rect.height()),
                );
                let shade = Color32::from_black_alpha(130);
                for band in [
                    Rect::from_min_max(rect.min, pos2(rect.right(), roi_rect.top())),
                    Rect::from_min_max(pos2(rect.left(), roi_rect.bottom()), rect.max),
                    Rect::from_min_max(
                        pos2(rect.left(), roi_rect.top()),
                        pos2(roi_rect.left(), roi_rect.bottom()),
                    ),
                    Rect::from_min_max(
                        pos2(roi_rect.right(), roi_rect.top()),
                        pos2(rect.right(), roi_rect.bottom()),
                    ),
                ] {
                    if band.is_positive() {
                        painter.rect_filled(band, 0.0, shade);
                    }
                }
                painter.rect_stroke(roi_rect, 2.0, Stroke::new(1.5, CYAN), StrokeKind::Inside);
            }
            None => {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if streaming {
                        "waiting for video"
                    } else {
                        "no video"
                    },
                    egui::FontId::proportional(13.0),
                    visuals.weak_text_color(),
                );
            }
        }
        painter.rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
            StrokeKind::Inside,
        );
        response
            .on_hover_text("Drag to set the region of interest, double-click for the full frame");
    }

    fn input_ui(&self, ui: &mut Ui, id: CameraId) {
        ui.vertical(|ui| {
            hint(ui, "model input");
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(INPUT_DISPLAY), Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
            if let Some(texture) = self.textures[id.index()].input.as_ref() {
                painter.image(
                    texture.id(),
                    rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        });
    }

    fn view_controls(ui: &mut Ui, id: CameraId, edited: &mut CameraConfig) {
        let view = edited.view_mut(id);
        ui.horizontal_wrapped(|ui| {
            ui.label("Rotation");
            ui.add(
                egui::Slider::new(&mut view.rotation, -180.0..=180.0)
                    .suffix("°")
                    .step_by(1.0),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut view.flip_x, "Flip horizontally");
            ui.checkbox(&mut view.flip_y, "Flip vertically");
            if ui
                .add_enabled(view.roi != Roi::default(), egui::Button::new("Full frame"))
                .clicked()
            {
                view.roi = Roi::default();
            }
        });
    }

    fn mouth_ui(
        &mut self,
        ui: &mut Ui,
        now: f64,
        camera: &CameraTelemetry,
        mouth: &MouthTelemetry,
        edited: &mut CameraConfig,
    ) {
        self.source_row(ui, CameraId::Mouth, "Source", camera, edited);
        self.model_row(
            ui,
            0,
            "path to faceModel.onnx",
            &mouth.model,
            &mut edited.mouth.model,
        );
        ui.add_space(6.0);
        ui.horizontal_top(|ui| {
            self.preview_ui(ui, CameraId::Mouth, camera, edited);
            self.input_ui(ui, CameraId::Mouth);
            ui.vertical(|ui| {
                Self::view_controls(ui, CameraId::Mouth, edited);
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if camera::mouth_calibrating() {
                        if ui
                            .add(egui::Button::new(RichText::new("Stop calibration").color(Color32::WHITE)).fill(PINK.gamma_multiply(0.7)))
                            .clicked()
                        {
                            let result = camera::stop_mouth_calibration();
                            self.report(now, result);
                        }
                        chip(ui, "calibrating", Some(PINK));
                    } else if ui
                        .button("Calibrate")
                        .on_hover_text("Record the range of every shape; make each mouth shape a few times, then stop")
                        .clicked()
                    {
                        camera::start_mouth_calibration();
                    }
                    if ui.add(icon_button(ui, &glyphs().reset, "Reset")).clicked() {
                        camera::reset_mouth_calibration();
                    }
                });
                filter_ui(ui, "mouth-filter", &mut edited.mouth.filter);
            });
        });
        ui.add_space(6.0);
        egui::CollapsingHeader::new("Mouth shapes")
            .id_salt("camera-mouth-shapes")
            .default_open(true)
            .show(ui, |ui| {
                hint(ui, "Bar = calibrated output, white mark = raw model value");
                egui::Grid::new("camera-mouth-grid")
                    .num_columns(6)
                    .spacing(vec2(8.0, 4.0))
                    .show(ui, |ui| {
                        for (index, name) in MOUTH_NAMES.iter().enumerate() {
                            ui.add_sized(
                                vec2(NAME_WIDTH, 16.0),
                                egui::Label::new(RichText::new(*name).small()).truncate(),
                            );
                            meter(
                                ui,
                                BAR_WIDTH,
                                mouth.output[index],
                                (0.0, 1.0),
                                Some(mouth.raw[index].clamp(0.0, 1.0)),
                                BLUE,
                            );
                            if index % 3 == 2 {
                                ui.end_row();
                            }
                        }
                    });
            });
    }

    fn eyes_ui(
        &mut self,
        ui: &mut Ui,
        now: f64,
        cameras: &[CameraTelemetry; 3],
        eyes: &EyesTelemetry,
        edited: &mut CameraConfig,
    ) {
        for (id, label) in [(CameraId::Left, "Left eye"), (CameraId::Right, "Right eye")] {
            self.source_row(ui, id, label, &cameras[id.index()], edited);
        }
        self.model_row(
            ui,
            1,
            "path to NEXT_ETVR*.onnx",
            &eyes.model,
            &mut edited.eyes.model,
        );
        ui.add_space(6.0);
        for id in [CameraId::Left, CameraId::Right] {
            ui.horizontal_top(|ui| {
                self.preview_ui(ui, id, &cameras[id.index()], edited);
                self.input_ui(ui, id);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(if id == CameraId::Left {
                            "Left eye"
                        } else {
                            "Right eye"
                        })
                        .strong(),
                    );
                    Self::view_controls(ui, id, edited);
                });
            });
            ui.add_space(4.0);
        }

        ui.horizontal_wrapped(|ui| {
            if camera::eyes_calibrating() {
                if ui
                    .add(egui::Button::new(RichText::new("Stop calibration").color(Color32::WHITE)).fill(PINK.gamma_multiply(0.7)))
                    .clicked()
                {
                    let result = camera::stop_eye_calibration();
                    self.report(now, result);
                }
                chip(ui, "calibrating", Some(PINK));
            } else if ui
                .button("Calibrate")
                .on_hover_text("Look at every edge of your view, blink, squint and raise your brows, then stop")
                .clicked()
            {
                camera::start_eye_calibration();
            }
            if ui
                .button("Recenter")
                .on_hover_text("Look straight ahead and click to make this the centre")
                .clicked()
            {
                camera::recenter_gaze();
                self.say(now, "Gaze recentred".into(), false);
            }
            if ui.add(icon_button(ui, &glyphs().reset, "Reset")).clicked() {
                camera::reset_eye_calibration();
            }
        });

        ui.add_space(6.0);
        ui.horizontal_top(|ui| {
            for (side, name) in [(0usize, "Left"), (1, "Right")] {
                ui.vertical(|ui| {
                    let present = eyes.present[side];
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name).strong());
                        if eyes.model.ready && !present {
                            chip(ui, "mirrored", Some(YELLOW));
                        }
                    });
                    let state = &eyes.state;
                    for (label, value, color) in [
                        ("closed", state.closed[side], PINK),
                        ("wide", state.wide[side], CYAN),
                        ("squint", state.squint[side], YELLOW),
                        ("brow up", state.brow[side], GREEN),
                    ] {
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                vec2(56.0, 16.0),
                                egui::Label::new(RichText::new(label).small()),
                            );
                            meter(ui, BAR_WIDTH, value, (0.0, 1.0), None, color);
                        });
                    }
                    let mut gaze = vec2(state.gaze_x[side], -state.gaze_y);
                    xy_pad(ui, 120.0, &mut gaze, false, BLUE);
                });
                ui.add_space(12.0);
            }
            ui.vertical(|ui| {
                let settings = &mut edited.eyes.settings;
                ui.label(RichText::new("Gaze").strong());
                egui::Grid::new("camera-gaze-grid")
                    .num_columns(3)
                    .spacing(vec2(8.0, 4.0))
                    .show(ui, |ui| {
                        for (label, axis) in [
                            ("left X", &mut settings.gaze.left_x),
                            ("right X", &mut settings.gaze.right_x),
                            ("Y", &mut settings.gaze.y),
                        ] {
                            ui.label(label);
                            ui.add(
                                egui::DragValue::new(&mut axis.gain)
                                    .speed(0.01)
                                    .range(0.1..=5.0)
                                    .prefix("gain "),
                            );
                            ui.add(
                                egui::DragValue::new(&mut axis.offset)
                                    .speed(0.005)
                                    .range(-1.0..=1.0)
                                    .prefix("offset "),
                            );
                            ui.end_row();
                        }
                    });
                ui.label(RichText::new("Limits").strong());
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut settings.limits.horizontal)
                            .range(1.0..=90.0)
                            .suffix("° side"),
                    );
                    ui.add(
                        egui::DragValue::new(&mut settings.limits.up)
                            .range(1.0..=90.0)
                            .suffix("° up"),
                    );
                    ui.add(
                        egui::DragValue::new(&mut settings.limits.down)
                            .range(1.0..=90.0)
                            .suffix("° down"),
                    );
                });
                ui.label(RichText::new("Eyelids").strong());
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut settings.open_lid)
                            .speed(0.005)
                            .range(0.05..=1.0)
                            .prefix("open at "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut settings.wide_lid)
                            .speed(0.005)
                            .range(0.05..=1.0)
                            .prefix("wide at "),
                    );
                });
                filter_ui(ui, "eye-filter", &mut edited.eyes.filter);
            });
        });

        egui::CollapsingHeader::new("Raw model outputs")
            .id_salt("camera-eye-raw")
            .show(ui, |ui| {
                egui::Grid::new("camera-eye-raw-grid")
                    .num_columns(4)
                    .spacing(vec2(8.0, 4.0))
                    .show(ui, |ui| {
                        for (index, name) in EYE_NAMES.iter().enumerate() {
                            let signed = name.starts_with("gaze");
                            ui.label(RichText::new(*name).small());
                            meter(
                                ui,
                                BAR_WIDTH,
                                eyes.raw[index],
                                if signed { (-1.0, 1.0) } else { (0.0, 1.0) },
                                None,
                                if signed { BLUE } else { PINK },
                            );
                            if index % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                let ranges = &edited.eyes.settings.expressions;
                for (index, name) in EXPRESSION_NAMES.iter().enumerate() {
                    hint(
                        ui,
                        &format!(
                            "{name}: {:.2} – {:.2}",
                            ranges.lower.get(index).copied().unwrap_or(0.0),
                            ranges.upper.get(index).copied().unwrap_or(1.0)
                        ),
                    );
                }
            });
    }

    fn update_textures(&mut self, ui: &Ui) {
        let raw: Vec<Option<Preview>> = {
            let previews = lock(&SHARED.previews);
            CameraId::ALL
                .iter()
                .map(|id| {
                    previews[id.index()]
                        .as_ref()
                        .filter(|p| p.generation != self.textures[id.index()].raw_generation)
                        .cloned()
                })
                .collect()
        };
        let inputs: Vec<Option<Preview>> = {
            let inputs = lock(&SHARED.inputs);
            CameraId::ALL
                .iter()
                .map(|id| {
                    inputs[id.index()]
                        .as_ref()
                        .filter(|p| p.generation != self.textures[id.index()].input_generation)
                        .cloned()
                })
                .collect()
        };
        for (id, (raw, input)) in CameraId::ALL.iter().zip(raw.into_iter().zip(inputs)) {
            let textures = &mut self.textures[id.index()];
            if let Some(preview) = raw {
                textures.raw_generation = preview.generation;
                textures.raw_size = [preview.width, preview.height];
                upload(
                    ui,
                    &mut textures.raw,
                    &format!("camera-raw-{}", id.index()),
                    &preview,
                );
            }
            if let Some(preview) = input {
                textures.input_generation = preview.generation;
                upload(
                    ui,
                    &mut textures.input,
                    &format!("camera-input-{}", id.index()),
                    &preview,
                );
            }
        }
    }
}

fn upload(ui: &Ui, slot: &mut Option<TextureHandle>, name: &str, preview: &Preview) {
    if preview.pixels.len() != preview.width * preview.height || preview.width == 0 {
        return;
    }
    let image = ColorImage::from_gray([preview.width, preview.height], &preview.pixels);
    match slot.as_mut() {
        Some(texture) => texture.set(image, TextureOptions::LINEAR),
        None => *slot = Some(ui.ctx().load_texture(name, image, TextureOptions::LINEAR)),
    }
}

fn filter_ui(ui: &mut Ui, id: &str, filter: &mut FilterParams) {
    egui::CollapsingHeader::new("Smoothing")
        .id_salt(id)
        .show(ui, |ui| {
            ui.checkbox(&mut filter.enabled, "One Euro filter");
            ui.add_enabled_ui(filter.enabled, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut filter.min_cutoff)
                            .speed(0.01)
                            .range(0.01..=10.0)
                            .prefix("min cutoff "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut filter.beta)
                            .speed(0.05)
                            .range(0.0..=50.0)
                            .prefix("beta "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut filter.d_cutoff)
                            .speed(0.01)
                            .range(0.01..=10.0)
                            .prefix("d cutoff "),
                    );
                });
                if ui.button("Defaults").clicked() {
                    *filter = FilterParams::default();
                }
            });
        });
}

fn commit(before: &CameraConfig, after: &CameraConfig) {
    camera::update_config(|config| {
        for id in CameraId::ALL {
            if before.source(id) != after.source(id) {
                config.set_source(id, after.source(id).map(str::to_string));
            }
            if before.view(id) != after.view(id) {
                *config.view_mut(id) = after.view(id).clone();
            }
        }
        if before.mouth.model != after.mouth.model {
            config.mouth.model = after.mouth.model.clone();
        }
        if before.eyes.model != after.eyes.model {
            config.eyes.model = after.eyes.model.clone();
        }
        if before.mouth.filter != after.mouth.filter {
            config.mouth.filter = after.mouth.filter;
        }
        if before.eyes.filter != after.eyes.filter {
            config.eyes.filter = after.eyes.filter;
        }
        let (old, new) = (&before.eyes.settings, &after.eyes.settings);
        let settings = &mut config.eyes.settings;
        if old.gaze != new.gaze {
            settings.gaze = new.gaze;
        }
        if old.limits != new.limits {
            settings.limits = new.limits;
        }
        if old.open_lid != new.open_lid || old.wide_lid != new.wide_lid {
            settings.open_lid = new.open_lid;
            settings.wide_lid = new.wide_lid;
        }
    });
}
