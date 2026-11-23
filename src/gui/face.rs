use eframe::egui;
use egui::{vec2, RichText, Ui};
use strum::{EnumCount, IntoEnumIterator};

use crate::{
    core::{
        ext_tracking::unified::{CombinedExpression, UnifiedExpressions, NUM_SHAPES},
        live::{Snapshot, LIVE},
    },
    FtMode,
};

use super::widgets::{
    card, chip, meter, meter_edit, pin_toggle, section, xy_pad, BLUE, CYAN, GREEN, MUTED, PINK,
    RED, YELLOW,
};

const GAZE_RANGE: f32 = 0.6;
const PAD: f32 = 130.0;
const SIGNED_UNIFIED: [UnifiedExpressions; 3] = [
    UnifiedExpressions::EyeLeftX,
    UnifiedExpressions::EyeRightX,
    UnifiedExpressions::EyeY,
];

const COLUMN_WIDTH: f32 = 380.0;
const NAME_WIDTH: f32 = 150.0;
const GROUPS: [(&str, &[&str]); 7] = [
    ("Eyes", &["Eye"]),
    ("Brows", &["Brow"]),
    ("Nose & cheeks", &["Nose", "Nasal", "Cheek"]),
    ("Jaw", &["Jaw"]),
    ("Mouth", &["Mouth"]),
    ("Lips", &["Lip"]),
    ("Tongue", &["Tongue"]),
];

struct Shape {
    index: usize,
    name: &'static str,
    lower: String,
    signed: bool,
}

struct Group {
    title: &'static str,
    shapes: Vec<Shape>,
    pinnable: bool,
}

pub struct FaceView {
    filter: String,
    active_only: bool,
    pinned_only: bool,
    groups: Vec<Group>,
}

impl FaceView {
    pub fn new() -> Self {
        let mut buckets: Vec<Vec<Shape>> = (0..=GROUPS.len()).map(|_| Vec::new()).collect();
        for e in UnifiedExpressions::iter() {
            let name: &'static str = e.into();
            let shape = Shape {
                index: e as usize,
                name,
                lower: name.to_lowercase(),
                signed: SIGNED_UNIFIED.iter().any(|s| *s as usize == e as usize),
            };
            let slot = GROUPS
                .iter()
                .position(|(_, prefixes)| prefixes.iter().any(|p| name.starts_with(p)))
                .unwrap_or(GROUPS.len());
            buckets[slot].push(shape);
        }
        let mut groups: Vec<Group> = buckets
            .into_iter()
            .enumerate()
            .filter(|(_, shapes)| !shapes.is_empty())
            .map(|(slot, shapes)| Group {
                title: GROUPS.get(slot).map_or("Other", |(title, _)| title),
                shapes,
                pinnable: true,
            })
            .collect();
        groups.push(Group {
            title: "Combined",
            shapes: CombinedExpression::iter()
                .map(|e| {
                    let name: &'static str = e.into();
                    Shape {
                        index: e as usize,
                        name,
                        lower: name.to_lowercase(),
                        signed: true,
                    }
                })
                .collect(),
            pinnable: false,
        });
        Self {
            filter: String::new(),
            active_only: false,
            pinned_only: false,
            groups,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, snapshot: &Snapshot) {
        let mut overrides = LIVE
            .controls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .overrides;
        let before = overrides;

        let gaze_width = 2.0 * PAD + 64.0;
        if ui.available_width() >= gaze_width + 420.0 {
            ui.horizontal_top(|ui| {
                let left_width =
                    ui.available_width() - gaze_width - ui.spacing().item_spacing.x * 2.0;
                ui.vertical(|ui| {
                    ui.set_width(left_width);
                    section(ui, "Output", |ui| self.output_ui(ui, snapshot));
                });
                ui.vertical(|ui| {
                    ui.set_width(gaze_width);
                    section(ui, "Gaze", |ui| gaze_ui(ui, snapshot));
                });
            });
        } else {
            section(ui, "Output", |ui| self.output_ui(ui, snapshot));
            ui.add_space(8.0);
            section(ui, "Gaze", |ui| gaze_ui(ui, snapshot));
        }

        ui.add_space(10.0);
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("Blendshapes")
                        .strong()
                        .size(15.0)
                        .color(ui.visuals().strong_text_color()),
                );
                ui.add_space(12.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("Filter shapes…")
                        .desired_width(200.0),
                );
                ui.checkbox(&mut self.active_only, "Active only");
                ui.checkbox(&mut self.pinned_only, "Pinned only");
                let pinned = overrides.iter().filter(|o| o.is_some()).count();
                if ui
                    .add_enabled(
                        pinned > 0,
                        egui::Button::new(format!("Unpin all ({pinned})")),
                    )
                    .clicked()
                {
                    overrides = [None; NUM_SHAPES];
                }
            });
            ui.label(
                RichText::new(
                    "Pin a shape to hold it at a fixed value regardless of tracking, then drag its bar. Useful for testing avatar blendshapes.",
                )
                .small()
                .weak(),
            );
        });
        ui.add_space(10.0);
        self.groups_ui(ui, snapshot, &mut overrides);

        if overrides != before {
            LIVE.controls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .overrides = overrides;
        }
    }

    fn groups_ui(
        &self,
        ui: &mut Ui,
        snapshot: &Snapshot,
        overrides: &mut [Option<f32>; NUM_SHAPES],
    ) {
        let filter = self.filter.to_lowercase();
        let pinned_mask = overrides.map(|o| o.is_some());
        let visible: Vec<(&Group, Vec<&Shape>)> = self
            .groups
            .iter()
            .filter(|group| group.pinnable || !self.pinned_only)
            .map(|group| {
                let shapes = group
                    .shapes
                    .iter()
                    .filter(|shape| {
                        (filter.is_empty() || shape.lower.contains(&filter))
                            && (!self.active_only || snapshot.shapes[shape.index].abs() > 0.01)
                            && (!self.pinned_only || pinned_mask[shape.index])
                    })
                    .collect::<Vec<_>>();
                (group, shapes)
            })
            .filter(|(_, shapes)| !shapes.is_empty())
            .collect();

        if visible.is_empty() {
            ui.label(RichText::new("No shapes match.").weak());
            return;
        }

        let gap = ui.spacing().item_spacing.x;
        let columns =
            (((ui.available_width() + gap) / (COLUMN_WIDTH + gap)).floor() as usize).clamp(1, 4);
        let mut heights = vec![0usize; columns];
        let mut assigned: Vec<Vec<usize>> = vec![Vec::new(); columns];
        for (index, (_, shapes)) in visible.iter().enumerate() {
            let column = (0..columns).min_by_key(|c| heights[*c]).unwrap_or(0);
            heights[column] += shapes.len() + 3;
            assigned[column].push(index);
        }

        ui.columns(columns, |cols| {
            for (col, indices) in cols.iter_mut().zip(&assigned) {
                for index in indices {
                    let (group, shapes) = &visible[*index];
                    section(
                        col,
                        &format!("{}  ·  {}", group.title, shapes.len()),
                        |ui| {
                            for shape in shapes {
                                shape_row(ui, shape, group.pinnable, snapshot, overrides);
                            }
                        },
                    );
                    col.add_space(10.0);
                }
            }
        });
        debug_assert_eq!(
            self.groups.iter().map(|g| g.shapes.len()).sum::<usize>(),
            UnifiedExpressions::COUNT + CombinedExpression::COUNT
        );
    }

    fn output_ui(&mut self, ui: &mut Ui, snapshot: &Snapshot) {
        egui::Grid::new("output-grid")
            .num_columns(2)
            .spacing(vec2(12.0, 8.0))
            .show(ui, |ui| {
                ui.label("Face tracking mode");
                let mut mode = snapshot.ft_mode;
                egui::ComboBox::from_id_salt("ft-mode")
                    .selected_text(ft_mode_name(mode))
                    .show_ui(ui, |ui| {
                        for option in [FtMode::Auto, FtMode::Legacy, FtMode::Native, FtMode::Both] {
                            ui.selectable_value(&mut mode, option, ft_mode_name(option))
                                .on_hover_text(ft_mode_help(option));
                        }
                    });
                if mode != snapshot.ft_mode {
                    LIVE.controls
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .ft_mode = Some(mode);
                }
                ui.end_row();

                ui.label("Avatar native FT");
                if snapshot.avatar_native_ft {
                    chip(ui, "supported", Some(GREEN));
                } else {
                    chip(ui, "not reported", Some(YELLOW));
                }
                ui.end_row();

                ui.label("Sending");
                ui.horizontal(|ui| {
                    if snapshot.sending_native {
                        chip(ui, "NATIVE /tracking/face/v1", Some(GREEN));
                    }
                    if snapshot.sending_legacy {
                        chip(ui, "LEGACY avatar parameters", Some(YELLOW));
                    }
                });
                ui.end_row();
            });
    }
}

fn shape_row(
    ui: &mut Ui,
    shape: &Shape,
    pinnable: bool,
    snapshot: &Snapshot,
    overrides: &mut [Option<f32>; NUM_SHAPES],
) {
    let range = if shape.signed {
        (-1.0, 1.0)
    } else {
        (0.0, 1.0)
    };
    let live = snapshot.shapes[shape.index];
    ui.horizontal(|ui| {
        let height = ui.spacing().interact_size.y;
        ui.allocate_ui_with_layout(
            vec2(NAME_WIDTH, height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(NAME_WIDTH);
                let color = if live.abs() > 0.01 {
                    ui.visuals().strong_text_color()
                } else {
                    MUTED
                };
                ui.add(
                    egui::Label::new(
                        RichText::new(shape.name)
                            .monospace()
                            .size(11.5)
                            .color(color),
                    )
                    .truncate(),
                )
                .on_hover_text(shape.name);
            },
        );
        let pin_width = if pinnable {
            22.0 + ui.spacing().item_spacing.x
        } else {
            0.0
        };
        let width = (ui.available_width() - pin_width).max(60.0);
        let color = if !pinnable {
            CYAN
        } else if shape.name.starts_with("Tongue") {
            PINK
        } else {
            BLUE
        };
        let pinned = &mut overrides[shape.index];
        match pinned.as_mut() {
            Some(value) if pinnable => {
                meter_edit(ui, width, value, range, YELLOW);
            }
            _ => {
                meter(ui, width, live, range, None, color);
            }
        }
        if pinnable {
            let mut is_pinned = pinned.is_some();
            if pin_toggle(ui, &mut is_pinned).changed() {
                *pinned = is_pinned.then_some(live.clamp(range.0, range.1));
            }
        }
    });
}

fn gaze_ui(ui: &mut Ui, snapshot: &Snapshot) {
    ui.horizontal(|ui| {
        for (label, eye) in [("Left", snapshot.eyes[0]), ("Right", snapshot.eyes[1])] {
            ui.vertical(|ui| {
                ui.set_width(PAD);
                ui.label(label);
                match eye {
                    Some(euler) => {
                        let mut dot = vec2(
                            (-euler.y / GAZE_RANGE).clamp(-1.0, 1.0),
                            (euler.x / GAZE_RANGE).clamp(-1.0, 1.0),
                        );
                        xy_pad(ui, PAD, &mut dot, false, CYAN);
                        ui.label(
                            RichText::new(format!(
                                "yaw {:+.1}° pitch {:+.1}°",
                                euler.y.to_degrees(),
                                euler.x.to_degrees()
                            ))
                            .monospace()
                            .small(),
                        );
                    }
                    None => {
                        let mut dot = vec2(0.0, 0.0);
                        xy_pad(ui, PAD, &mut dot, false, RED.gamma_multiply(0.4));
                        ui.label(RichText::new("no gaze").small().color(RED));
                    }
                }
            });
        }
    });
}

fn ft_mode_name(mode: FtMode) -> &'static str {
    match mode {
        FtMode::Auto => "Auto",
        FtMode::Legacy => "Legacy",
        FtMode::Native => "Native",
        FtMode::Both => "Both",
    }
}

fn ft_mode_help(mode: FtMode) -> &'static str {
    match mode {
        FtMode::Auto => {
            "Native when the avatar reports Use All Tracking Keys, legacy parameters otherwise"
        }
        FtMode::Legacy => "Only send VRCFT-style avatar parameters",
        FtMode::Native => "Only send VRChat native face tracking",
        FtMode::Both => "Send native face tracking and avatar parameters",
    }
}
