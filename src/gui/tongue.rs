use eframe::egui;
use std::{collections::VecDeque, sync::atomic::Ordering};

use egui::{vec2, Color32, ColorImage, RichText, TextureHandle, TextureOptions, Ui, Vec2};

use crate::core::{
    ext_tracking::{
        qpro_tongue::{
            relay, shape_expressions, Telemetry, Tuning, Visibility, PREVIEW_SIZE, SHAPE_COUNT,
            SHARED,
        },
        unified::UnifiedExpressions,
    },
    live::LIVE,
};

use super::{
    assets::glyphs,
    widgets::{
        chip, history_plot, icon_button, legend, meter, primary_button, section, xy_pad, Trace,
        BLUE, CYAN, GREEN, PINK, RED, YELLOW,
    },
};

const HISTORY_SECONDS: f64 = 12.0;
const CAMERA_DISPLAY: f32 = 180.0;

#[derive(Clone, Copy)]
struct Sample {
    camera: f32,
    native: f32,
    fused: f32,
    threshold: f32,
    visible: bool,
}

struct Message {
    text: String,
    error: bool,
    at: f64,
}

pub struct TongueView {
    times: VecDeque<f64>,
    samples: VecDeque<Sample>,
    textures: Option<[TextureHandle; 2]>,
    generation: u64,
    show_camera: bool,
    saved: Option<Tuning>,
    message: Option<Message>,
    manual: [f32; SHAPE_COUNT],
    names: [&'static str; SHAPE_COUNT],
    indices: [usize; SHAPE_COUNT],
    up: usize,
    down: usize,
    left: usize,
    right: usize,
}

impl TongueView {
    pub fn new() -> Self {
        let expressions = shape_expressions();
        let position = |target: UnifiedExpressions| {
            expressions
                .iter()
                .position(|e| *e as usize == target as usize)
                .unwrap()
        };
        Self {
            times: VecDeque::new(),
            samples: VecDeque::new(),
            textures: None,
            generation: 0,
            show_camera: true,
            saved: Tuning::load_saved().ok().flatten(),
            message: None,
            manual: [0.0; SHAPE_COUNT],
            names: expressions.map(|e| e.into()),
            indices: expressions.map(|e| e as usize),
            up: position(UnifiedExpressions::TongueUp),
            down: position(UnifiedExpressions::TongueDown),
            left: position(UnifiedExpressions::TongueLeft),
            right: position(UnifiedExpressions::TongueRight),
        }
    }

    pub fn wants_preview(&self) -> bool {
        self.show_camera
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let active = SHARED.active.load(Ordering::Relaxed);
        let telemetry = SHARED
            .telemetry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let now = ui.input(|i| i.time);
        self.record(now, &telemetry);
        self.update_textures(ui);

        let mut overrides = LIVE
            .controls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .overrides;
        let overrides_before = overrides;
        let mut manual_on = self.indices.iter().all(|&i| overrides[i].is_some());
        if manual_on {
            for (value, &index) in self.manual.iter_mut().zip(&self.indices) {
                *value = overrides[index].unwrap_or(0.0);
            }
        }

        let mut tuning = SHARED
            .tuning
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let tuning_before = tuning.clone();

        self.header_ui(ui, active, &telemetry, manual_on);
        ui.add_space(6.0);
        if active {
            relay_ui(ui, &telemetry);
        }

        if active {
            let right_width = 2.0 * CAMERA_DISPLAY + 48.0;
            if ui.available_width() >= right_width + 560.0 {
                ui.horizontal_top(|ui| {
                    let left_width =
                        ui.available_width() - right_width - ui.spacing().item_spacing.x;
                    ui.vertical(|ui| {
                        ui.set_width(left_width);
                        section(ui, "Detection", |ui| {
                            detection_ui(ui, &mut tuning, &telemetry)
                        });
                        ui.add_space(6.0);
                        section(ui, "Scores", |ui| self.scores_ui(ui, &telemetry));
                    });
                    ui.vertical(|ui| {
                        section(ui, "Mouth cameras", |ui| self.camera_ui(ui, &telemetry));
                    });
                });
            } else {
                section(ui, "Detection", |ui| {
                    detection_ui(ui, &mut tuning, &telemetry)
                });
                ui.add_space(6.0);
                section(ui, "Scores", |ui| self.scores_ui(ui, &telemetry));
                ui.add_space(6.0);
                section(ui, "Mouth cameras", |ui| self.camera_ui(ui, &telemetry));
            }
            ui.add_space(6.0);
        } else {
            section(ui, "Camera tongue tracking is not running", |ui| {
                ui.label(
                    "Start Visage with --tongue-model (for example TONGUE_ENABLED=1 ./start.sh) to see live scores and tune detection here.",
                );
                ui.label("Manual tongue sliders below still work and override whatever the headset sends.");
            });
            ui.add_space(6.0);
        }

        section(ui, "Tongue shapes", |ui| {
            ui.horizontal(|ui| {
                if ui
                    .checkbox(&mut manual_on, "Manual override")
                    .on_hover_text("Drive every tongue shape from the sliders below, ignoring the camera and headset")
                    .changed()
                {
                    for (&index, value) in self.indices.iter().zip(&self.manual) {
                        overrides[index] = manual_on.then_some(*value);
                    }
                }
                if manual_on && ui.button("Zero").clicked() {
                    self.manual = [0.0; SHAPE_COUNT];
                }
                if active && ui.button("Reset gains").clicked() {
                    tuning.gains = [1.0; SHAPE_COUNT];
                }
            });
            ui.add_space(4.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| self.shapes_grid(ui, active, manual_on, &mut tuning, &telemetry));
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(if manual_on {
                        "Direction (drag, double-click to centre)"
                    } else {
                        "Direction"
                    });
                    let mut direction = if manual_on {
                        vec2(
                            self.manual[self.right] - self.manual[self.left],
                            self.manual[self.up] - self.manual[self.down],
                        )
                    } else {
                        vec2(telemetry.horizontal, telemetry.vertical)
                    };
                    let color = if manual_on || telemetry.visible {
                        PINK
                    } else {
                        PINK.gamma_multiply(0.35)
                    };
                    if xy_pad(ui, 160.0, &mut direction, manual_on, color).changed() {
                        self.set_direction(direction);
                    }
                });
            });
            if manual_on {
                for (&index, value) in self.indices.iter().zip(&self.manual) {
                    overrides[index] = Some(*value);
                }
            }
        });

        if active {
            ui.add_space(6.0);
            self.persistence_ui(ui, &mut tuning, now);
        }

        if tuning != tuning_before {
            *SHARED.tuning.lock().unwrap_or_else(|e| e.into_inner()) = tuning;
        }
        if overrides != overrides_before {
            LIVE.controls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .overrides = overrides;
        }
    }

    fn set_direction(&mut self, direction: Vec2) {
        self.manual[self.right] = direction.x.max(0.0);
        self.manual[self.left] = (-direction.x).max(0.0);
        self.manual[self.up] = direction.y.max(0.0);
        self.manual[self.down] = (-direction.y).max(0.0);
    }

    fn record(&mut self, now: f64, telemetry: &Telemetry) {
        if telemetry.fresh {
            self.times.push_back(now);
            self.samples.push_back(Sample {
                camera: telemetry.camera,
                native: telemetry.native,
                fused: telemetry.fused,
                threshold: telemetry.threshold,
                visible: telemetry.visible,
            });
        }
        while self
            .times
            .front()
            .is_some_and(|&t| now - t > HISTORY_SECONDS)
        {
            self.times.pop_front();
            self.samples.pop_front();
        }
    }

    fn update_textures(&mut self, ui: &Ui) {
        let size = [PREVIEW_SIZE, PREVIEW_SIZE];
        let images = {
            let preview = SHARED.preview.lock().unwrap_or_else(|e| e.into_inner());
            let Some(preview) = preview.as_ref() else {
                return;
            };
            if preview.generation == self.generation {
                return;
            }
            self.generation = preview.generation;
            [
                ColorImage::from_gray(size, &preview.left),
                ColorImage::from_gray(size, &preview.right),
            ]
        };
        match self.textures.as_mut() {
            Some(textures) => {
                for (texture, image) in textures.iter_mut().zip(images) {
                    texture.set(image, TextureOptions::LINEAR);
                }
            }
            None => {
                let [left, right] = images;
                self.textures = Some([
                    ui.ctx()
                        .load_texture("tongue-left", left, TextureOptions::LINEAR),
                    ui.ctx()
                        .load_texture("tongue-right", right, TextureOptions::LINEAR),
                ]);
            }
        }
    }

    fn header_ui(&self, ui: &mut Ui, active: bool, telemetry: &Telemetry, manual_on: bool) {
        ui.horizontal_wrapped(|ui| {
            if manual_on {
                chip(ui, "MANUAL OVERRIDE", Some(YELLOW));
            }
            if !active {
                chip(ui, "camera model off", None);
                return;
            }
            if let Some(error) = telemetry.error.as_ref() {
                chip(ui, "model error", Some(RED)).on_hover_text(error);
            } else if telemetry.ready {
                chip(ui, "models ready", Some(GREEN));
            } else {
                chip(ui, "loading models", Some(YELLOW));
            }
            if telemetry.connected {
                chip(ui, "camera stream", Some(GREEN));
            } else {
                chip(ui, "no camera stream", Some(RED)).on_hover_text(
                    "Start the headset cameras below, or check that the relay is running and the adb port is forwarded",
                );
            }
            if telemetry.native_fresh {
                chip(ui, "headset tongue", Some(GREEN));
            } else {
                chip(ui, "no headset tongue", Some(YELLOW))
                    .on_hover_text("WiVRn face tracking is not supplying a native tongue score");
            }
            if telemetry.fresh {
                if telemetry.visible {
                    chip(ui, "TONGUE OUT", Some(PINK));
                } else {
                    chip(ui, "tongue in", None);
                }
                chip(
                    ui,
                    &format!("{:.0} fps · {:.1} ms", telemetry.fps, telemetry.latency_ms),
                    None,
                );
            }
        });
    }

    fn scores_ui(&self, ui: &mut Ui, telemetry: &Telemetry) {
        let width = (ui.available_width() - 110.0).max(120.0);
        egui::Grid::new("tongue-scores")
            .num_columns(2)
            .spacing(vec2(10.0, 6.0))
            .show(ui, |ui| {
                ui.label("Camera");
                meter(
                    ui,
                    width,
                    telemetry.camera,
                    (0.0, 1.0),
                    Some(telemetry.threshold),
                    BLUE,
                );
                ui.end_row();
                ui.label("Headset");
                meter(ui, width, telemetry.native, (0.0, 1.0), None, YELLOW);
                ui.end_row();
                ui.label(RichText::new("Fused").strong());
                let color = if telemetry.visible {
                    PINK
                } else {
                    PINK.gamma_multiply(0.5)
                };
                meter(
                    ui,
                    width,
                    telemetry.fused,
                    (0.0, 1.0),
                    Some(telemetry.threshold),
                    color,
                )
                .on_hover_text("White marker is the activation threshold");
                ui.end_row();
                ui.label("Extension");
                meter(ui, width, telemetry.raw[0], (0.0, 1.0), None, CYAN);
                ui.end_row();
            });
        ui.add_space(6.0);
        let samples = &self.samples;
        history_plot(
            ui,
            110.0,
            &self.times,
            HISTORY_SECONDS,
            &|i| samples[i].visible,
            &[
                Trace {
                    values: &|i| samples[i].camera,
                    color: BLUE,
                    width: 1.5,
                    dashed: false,
                },
                Trace {
                    values: &|i| samples[i].native,
                    color: YELLOW,
                    width: 1.5,
                    dashed: false,
                },
                Trace {
                    values: &|i| samples[i].fused,
                    color: PINK,
                    width: 2.0,
                    dashed: false,
                },
                Trace {
                    values: &|i| samples[i].threshold,
                    color: Color32::WHITE,
                    width: 1.0,
                    dashed: true,
                },
            ],
        );
        legend(
            ui,
            &[
                ("camera", BLUE),
                ("headset", YELLOW),
                ("fused", PINK),
                ("threshold", Color32::WHITE),
            ],
        );
        ui.label(
            RichText::new("Shaded = tongue active. Pick a threshold above your smile/talking scores and below tongue-out scores.")
                .small()
                .weak(),
        );
    }

    fn camera_ui(&mut self, ui: &mut Ui, telemetry: &Telemetry) {
        ui.checkbox(&mut self.show_camera, "Show camera feed");
        if !self.show_camera {
            return;
        }
        match self.textures.as_ref() {
            Some(textures) if telemetry.connected => {
                ui.horizontal(|ui| {
                    for texture in textures {
                        ui.add(
                            egui::Image::new(texture)
                                .fit_to_exact_size(Vec2::splat(CAMERA_DISPLAY))
                                .corner_radius(4.0),
                        );
                    }
                });
            }
            _ => {
                let (rect, _) = ui.allocate_exact_size(
                    vec2(
                        2.0 * CAMERA_DISPLAY + ui.spacing().item_spacing.x,
                        CAMERA_DISPLAY,
                    ),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "waiting for frames…",
                    egui::FontId::proportional(13.0),
                    ui.visuals().weak_text_color(),
                );
            }
        }
    }

    fn shapes_grid(
        &mut self,
        ui: &mut Ui,
        active: bool,
        manual_on: bool,
        tuning: &mut Tuning,
        telemetry: &Telemetry,
    ) {
        let live = LIVE
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .shapes;
        egui::Grid::new("tongue-shapes")
            .num_columns(5)
            .striped(true)
            .spacing(vec2(10.0, 4.0))
            .show(ui, |ui| {
                ui.label(RichText::new("Shape").strong());
                ui.label(RichText::new("Model").strong())
                    .on_hover_text("Camera model output before gain");
                ui.label(RichText::new("Sent").strong())
                    .on_hover_text("Value actually sent to the avatar");
                ui.label(RichText::new("Gain").strong());
                ui.label(RichText::new("Manual").strong());
                ui.end_row();
                for index in 0..SHAPE_COUNT {
                    ui.label(RichText::new(self.names[index]).monospace());
                    let model = if active { telemetry.raw[index] } else { 0.0 };
                    meter(ui, 110.0, model, (0.0, 1.0), None, BLUE.gamma_multiply(0.8));
                    meter(ui, 110.0, live[self.indices[index]], (0.0, 1.0), None, PINK);
                    ui.add_enabled(
                        active && !manual_on,
                        egui::Slider::new(&mut tuning.gains[index], 0.0..=3.0)
                            .fixed_decimals(2)
                            .suffix("×"),
                    );
                    ui.add_enabled(
                        manual_on,
                        egui::Slider::new(&mut self.manual[index], 0.0..=1.0).fixed_decimals(2),
                    );
                    ui.end_row();
                }
            });
    }

    fn persistence_ui(&mut self, ui: &mut Ui, tuning: &mut Tuning, now: f64) {
        ui.horizontal(|ui| {
            let dirty = self.saved.as_ref() != Some(tuning);
            if ui
                .add(icon_button(
                    ui,
                    &glyphs().save,
                    if dirty {
                        "Save tuning •"
                    } else {
                        "Save tuning"
                    },
                ))
                .on_hover_text(format!(
                    "Write to {}. Loaded automatically on the next start.",
                    Tuning::path().display()
                ))
                .clicked()
            {
                self.message = Some(match tuning.save() {
                    Ok(path) => {
                        self.saved = Some(tuning.clone());
                        Message {
                            text: format!("Saved to {}", path.display()),
                            error: false,
                            at: now,
                        }
                    }
                    Err(e) => Message {
                        text: format!("Save failed: {e:#}"),
                        error: true,
                        at: now,
                    },
                });
            }
            if ui
                .add_enabled(
                    self.saved.is_some() && dirty,
                    icon_button(ui, &glyphs().revert, "Revert to saved"),
                )
                .clicked()
            {
                if let Some(saved) = self.saved.as_ref() {
                    *tuning = saved.clone();
                }
            }
            if ui
                .add(icon_button(ui, &glyphs().headset, "Launch values"))
                .on_hover_text("Values from the command line / start.sh environment")
                .clicked()
            {
                *tuning = SHARED
                    .launch
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
            }
            if ui
                .add(icon_button(ui, &glyphs().reset, "Defaults"))
                .on_hover_text("Model threshold and weight, smoothing 55, unity gains")
                .clicked()
            {
                *tuning = Tuning::default();
            }
            if self.saved.is_some()
                && ui
                    .add(icon_button(ui, &glyphs().trash, "Delete saved"))
                    .on_hover_text("Remove the saved file so start.sh options apply again")
                    .clicked()
            {
                self.message = Some(match std::fs::remove_file(Tuning::path()) {
                    Ok(()) => {
                        self.saved = None;
                        Message {
                            text: "Saved tuning removed".into(),
                            error: false,
                            at: now,
                        }
                    }
                    Err(e) => Message {
                        text: format!("Remove failed: {e}"),
                        error: true,
                        at: now,
                    },
                });
            }
            if let Some(message) = self.message.as_ref() {
                if now - message.at < 6.0 {
                    ui.label(RichText::new(&message.text).color(if message.error {
                        RED
                    } else {
                        GREEN
                    }));
                }
            }
        });
    }
}

fn relay_ui(ui: &mut Ui, telemetry: &Telemetry) {
    if telemetry.connected && telemetry.fresh {
        return;
    }
    let status = relay::status();
    let Some(script) = status.script.as_ref() else {
        let text = match status.missing.as_ref() {
            Some(path) => format!(
                "Headset relay script not found at {}. Run ./headset-stream.sh start by hand.",
                path.display()
            ),
            None => "Start the headset cameras with ./headset-stream.sh start, or launch from the Visage desktop entry (or ./start.sh) to get a start button here.".to_string(),
        };
        ui.label(RichText::new(text).weak());
        ui.add_space(6.0);
        return;
    };
    section(ui, "Headset cameras", |ui| {
        ui.horizontal_wrapped(|ui| {
            let ready = status.headset.ready();
            if status.busy {
                ui.spinner();
                ui.label("Starting the mouth-camera relay on the headset…");
            } else if ui
                .add_enabled(ready, primary_button(&glyphs().camera, "Start headset cameras"))
                .on_hover_text(format!(
                    "Runs {} start: checks root, pushes the relay, injects the camera streamer and forwards the adb port",
                    script.display()
                ))
                .on_disabled_hover_text(status.headset.describe())
                .clicked()
            {
                relay::start();
            }
            if !status.busy {
                let color = if ready { GREEN } else { YELLOW };
                ui.label(RichText::new(status.headset.describe()).color(color));
            }
            if !status.busy && status.watching && ready {
                let text = match status.retry_in {
                    Some(wait) if wait.as_secs() > 0 => {
                        format!("Retrying automatically in {}s", wait.as_secs())
                    }
                    _ => "Retrying automatically".to_string(),
                };
                ui.label(RichText::new(text).weak());
            }
        });
        match status.last.as_ref() {
            Some(outcome) if !status.busy && !outcome.ok => {
                let reason = outcome
                    .output
                    .lines()
                    .rev()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("no output");
                ui.label(RichText::new(format!("Start failed: {reason}")).color(RED))
                    .on_hover_text(&outcome.output);
                ui.label(
                    RichText::new("The headset must be connected over adb and rooted (Singularity, Magisk shell root granted).")
                        .small()
                        .weak(),
                );
            }
            Some(outcome) if !status.busy && outcome.ok => {
                ui.label(
                    RichText::new("Relay started, waiting for the camera stream…").color(YELLOW),
                );
            }
            _ => {}
        }
    });
    ui.add_space(6.0);
}

fn detection_ui(ui: &mut Ui, tuning: &mut Tuning, telemetry: &Telemetry) {
    let slider_width = (ui.available_width() - 360.0).clamp(100.0, 380.0);
    ui.spacing_mut().slider_width = slider_width;
    let effective_threshold = tuning.threshold.unwrap_or(telemetry.model_threshold);
    egui::Grid::new("tongue-detection")
        .num_columns(3)
        .spacing(vec2(10.0, 8.0))
        .show(ui, |ui| {
            ui.label("Camera tongue");
            ui.checkbox(&mut tuning.enabled, "Apply")
                .on_hover_text("Off passes the headset's own tongue value through untouched");
            ui.label("");
            ui.end_row();

            ui.label("Visibility");
            egui::ComboBox::from_id_salt("tongue-visibility")
                .selected_text(visibility_name(tuning.visibility))
                .show_ui(ui, |ui| {
                    for mode in [Visibility::Weighted, Visibility::Camera, Visibility::Native, Visibility::Agreement] {
                        ui.selectable_value(&mut tuning.visibility, mode, visibility_name(mode))
                            .on_hover_text(visibility_help(mode));
                    }
                });
            ui.label("");
            ui.end_row();

            ui.label("Threshold");
            let threshold_slider =
                |value| egui::Slider::new(value, 0.01..=1.0).fixed_decimals(3).step_by(0.005);
            match tuning.threshold.as_mut() {
                Some(threshold) => {
                    ui.add(threshold_slider(threshold));
                }
                None => {
                    let mut threshold = telemetry.model_threshold;
                    ui.add_enabled(false, threshold_slider(&mut threshold));
                }
            }
            let mut model = tuning.threshold.is_none();
            if ui
                .checkbox(&mut model, format!("model ({:.3})", telemetry.model_threshold))
                .changed()
            {
                tuning.threshold = (!model).then_some(telemetry.model_threshold);
            }
            ui.end_row();

            ui.label("Camera weight");
            let weighted = tuning.visibility == Visibility::Weighted;
            let weight_hover = "Weighted mode: fused = weight × camera + (1 − weight) × headset";
            match tuning.camera_weight.as_mut() {
                Some(weight) if weighted => {
                    ui.add(egui::Slider::new(weight, 0.0..=1.0).fixed_decimals(2))
                        .on_hover_text(weight_hover);
                }
                current => {
                    let mut weight = current.map_or(telemetry.model_camera_weight, |w| *w);
                    ui.add_enabled(false, egui::Slider::new(&mut weight, 0.0..=1.0).fixed_decimals(2))
                        .on_hover_text(weight_hover);
                }
            }
            let mut model = tuning.camera_weight.is_none();
            if ui
                .add_enabled(
                    weighted,
                    egui::Checkbox::new(&mut model, format!("model ({:.2})", telemetry.model_camera_weight)),
                )
                .changed()
            {
                tuning.camera_weight = (!model).then_some(telemetry.model_camera_weight);
            }
            ui.end_row();

            ui.label("Release");
            ui.add(egui::Slider::new(&mut tuning.release, 0.0..=0.3).fixed_decimals(3))
                .on_hover_text("Hysteresis: once active, the tongue stays out until the fused score drops this far below the threshold");
            ui.label(RichText::new(format!("off below {:.3}", (effective_threshold - tuning.release).max(0.0))).small().weak());
            ui.end_row();

            ui.label("Smoothing");
            ui.add(egui::Slider::new(&mut tuning.smoothing, 0..=100))
                .on_hover_text("0 = most responsive, 100 = smoothest");
            ui.label("");
            ui.end_row();
        });

    let threshold = tuning.threshold.unwrap_or(telemetry.model_threshold);
    let weight = tuning
        .camera_weight
        .unwrap_or(telemetry.model_camera_weight);
    let warning = match tuning.visibility {
        Visibility::Weighted if threshold >= weight && telemetry.native < 0.01 => Some(format!(
            "With the headset score at 0 the fused score tops out at {weight:.2}, so a threshold of {threshold:.3} can never activate."
        )),
        Visibility::Agreement | Visibility::Native if !telemetry.native_fresh || telemetry.native < 0.01 => Some(
            "This mode needs the headset's own tongue score, which is currently 0.".to_string(),
        ),
        _ => None,
    };
    if let Some(warning) = warning {
        ui.add_space(4.0);
        ui.label(RichText::new(format!("⚠ {warning}")).color(YELLOW));
    }
}

fn visibility_name(mode: Visibility) -> &'static str {
    match mode {
        Visibility::Weighted => "Weighted",
        Visibility::Camera => "Camera",
        Visibility::Native => "Headset",
        Visibility::Agreement => "Agreement",
    }
}

fn visibility_help(mode: Visibility) -> &'static str {
    match mode {
        Visibility::Weighted => "Blend camera and headset scores by the camera weight",
        Visibility::Camera => "Camera model score only",
        Visibility::Native => "Headset tongue score only",
        Visibility::Agreement => "Both must agree (lower of the two)",
    }
}
