use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
};

use eframe::{egui, egui_glow};
use egui::{vec2, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind};
use glam::{vec2 as gvec2, Mat4, Vec2, Vec3};
use strum::IntoEnumIterator;

use crate::core::{
    ext_tracking::unified::{UnifiedExpressions, NUM_SHAPES},
    live::LIVE,
};

use self::{
    model::{signed, unit, Jaw, Lid, Model, Morph, Pose, Side, Tongue, VIEW_TARGET},
    render::{EyeDraw, Frame, Shared},
};
use super::widgets::{chip, BLUE, CARD, CARD_EDGE, CYAN, PINK, RED, YELLOW};

mod model;
mod render;

const GAZE_MAX: f32 = std::f32::consts::FRAC_PI_6;
const FOV: f32 = 0.49;
const IRIS: Vec3 = Vec3::new(0.10, 0.42, 0.50);
const TOP_SHAPES: usize = 6;

#[derive(Clone, Copy, PartialEq)]
struct Camera {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.12,
            pitch: 0.08,
            distance: 5.9,
        }
    }
}

impl Camera {
    fn position(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        VIEW_TARGET + Vec3::new(sy * cp, sp, cy * cp) * self.distance
    }
}

enum Loader {
    Idle,
    Building(mpsc::Receiver<Model>),
    Ready(Arc<Model>),
    Failed(String),
}

pub struct Face3d {
    camera: Camera,
    wireframe: bool,
    loader: Loader,
    shared: Arc<Mutex<Shared>>,
    last_pose: Option<Pose>,
}

impl Face3d {
    pub fn new() -> Self {
        Self {
            camera: Camera::default(),
            wireframe: false,
            loader: Loader::Idle,
            shared: Arc::new(Mutex::new(Shared::default())),
            last_pose: None,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shapes: &[f32; NUM_SHAPES], eyes: [Option<Vec3>; 2]) {
        self.poll_loader();
        let pinned = LIVE
            .controls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .overrides;
        let gaze_pinned = [
            UnifiedExpressions::EyeLeftX,
            UnifiedExpressions::EyeRightX,
            UnifiedExpressions::EyeY,
        ]
        .iter()
        .any(|e| pinned[*e as usize].is_some());
        let gaze = gaze_angles(shapes, eyes, gaze_pinned);
        let pose = pose_from_shapes(shapes, &gaze);

        egui::Frame::new()
            .fill(CARD)
            .stroke(Stroke::new(1.0, CARD_EDGE))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.toolbar(ui, shapes, &pinned);
                ui.add_space(6.0);
                self.viewport(ui, &pose, &gaze);
            });
    }

    fn poll_loader(&mut self) {
        match &self.loader {
            Loader::Idle => {
                let (tx, rx) = mpsc::channel();
                let spawned = thread::Builder::new()
                    .name("face3d-mesh".into())
                    .spawn(move || {
                        let _ = tx.send(Model::build());
                    });
                self.loader = match spawned {
                    Ok(_) => Loader::Building(rx),
                    Err(e) => Loader::Failed(format!("could not start mesh builder: {e}")),
                };
            }
            Loader::Building(rx) => match rx.try_recv() {
                Ok(model) => {
                    let model = Arc::new(model);
                    self.shared
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .set_model(model.clone());
                    self.last_pose = None;
                    self.loader = Loader::Ready(model);
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.loader = Loader::Failed("mesh builder stopped unexpectedly".into());
                }
            },
            Loader::Ready(_) | Loader::Failed(_) => {}
        }
    }

    fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        shapes: &[f32; NUM_SHAPES],
        pinned: &[Option<f32>; NUM_SHAPES],
    ) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.camera != Camera::default(),
                    egui::Button::new("Reset view"),
                )
                .on_hover_text("Double-click the preview to reset as well")
                .clicked()
            {
                self.camera = Camera::default();
            }
            ui.checkbox(&mut self.wireframe, "Wireframe");
            ui.separator();
            let top = top_shapes(shapes);
            if top.is_empty() {
                ui.label(RichText::new("Neutral: no active shapes").weak());
            }
            for (shape, value) in top {
                if ui.available_width() < 190.0 {
                    break;
                }
                let name: &'static str = shape.into();
                let color = if pinned[shape as usize].is_some() {
                    YELLOW
                } else if name.starts_with("Tongue") {
                    PINK
                } else if name.starts_with("Eye") {
                    CYAN
                } else {
                    BLUE
                };
                chip(ui, &format!("{name} {value:+.2}"), Some(color));
            }
        });
    }

    fn viewport(&mut self, ui: &mut egui::Ui, pose: &Pose, gaze: &[Vec2; 2]) {
        let size = vec2(
            ui.available_width().max(160.0),
            ui.available_height().max(220.0),
        );
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());

        if response.dragged() {
            let d = response.drag_delta();
            self.camera.yaw -= d.x * 0.008;
            self.camera.pitch = (self.camera.pitch + d.y * 0.008).clamp(-1.2, 1.2);
        }
        if response.hovered() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            if scroll != 0.0 {
                self.camera.distance *= (-scroll * 0.0015).exp();
            }
            if zoom.is_finite() && zoom > 0.0 && zoom != 1.0 {
                self.camera.distance /= zoom;
            }
            self.camera.distance = self.camera.distance.clamp(2.4, 12.0);
        }
        if response.double_clicked() {
            self.camera = Camera::default();
        }
        if !self.camera.yaw.is_finite()
            || !self.camera.pitch.is_finite()
            || !self.camera.distance.is_finite()
        {
            self.camera = Camera::default();
        }
        self.camera.yaw = self.camera.yaw.rem_euclid(std::f32::consts::TAU);

        if !ui.is_rect_visible(rect) {
            return;
        }
        let painter = ui.painter_at(rect);
        let dark = ui.visuals().dark_mode;
        paint_background(&painter, rect, dark);

        let text_color = if dark {
            Color32::from_gray(200)
        } else {
            Color32::from_gray(60)
        };
        match &self.loader {
            Loader::Ready(model) => {
                let model = model.clone();
                let error = self
                    .shared
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .error()
                    .map(str::to_owned);
                match error {
                    Some(e) => {
                        painter.text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            format!("3D preview unavailable\n{e}"),
                            FontId::proportional(14.0),
                            RED,
                        );
                    }
                    None => self.paint_model(&painter, rect, &model, pose, gaze),
                }
            }
            Loader::Building(_) | Loader::Idle => {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "Sculpting head…",
                    FontId::proportional(14.0),
                    text_color,
                );
                ui.ctx().request_repaint();
            }
            Loader::Failed(e) => {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    format!("3D preview unavailable\n{e}"),
                    FontId::proportional(14.0),
                    RED,
                );
            }
        }

        painter.text(
            rect.left_bottom() + vec2(10.0, -8.0),
            Align2::LEFT_BOTTOM,
            "Drag to orbit · Scroll to zoom · Double-click to reset",
            FontId::proportional(11.5),
            text_color.gamma_multiply(0.7),
        );
        painter.rect_stroke(rect, 8.0, Stroke::new(1.0, CARD_EDGE), StrokeKind::Inside);
    }

    fn paint_model(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        model: &Arc<Model>,
        pose: &Pose,
        gaze: &[Vec2; 2],
    ) {
        let changed = self
            .last_pose
            .is_none_or(|last| last.difference(pose) > 1e-4);
        if changed {
            let mut vertices = Vec::with_capacity(model.vertex_count * model::STRIDE);
            model.pose_vertices(pose, &mut vertices);
            self.shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .submit(vertices);
            self.last_pose = Some(*pose);
        }

        let aspect = (rect.width() / rect.height().max(1.0)).clamp(0.1, 10.0);
        let camera = self.camera.position();
        let view = Mat4::look_at_rh(camera, VIEW_TARGET, Vec3::Y);
        let proj = Mat4::perspective_rh_gl(FOV, aspect, 0.1, 50.0);
        let forward = (VIEW_TARGET - camera).normalize_or(Vec3::NEG_Z);
        let right = forward.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(forward);
        let back = -forward;
        let eyes = [0, 1].map(|i| {
            let center = model.eyes[i];
            let g = gaze[i];
            EyeDraw {
                model: Mat4::from_translation(center)
                    * Mat4::from_rotation_y(g.x)
                    * Mat4::from_rotation_x(-g.y),
                center,
                lid_upper: pose.lids[i].upper,
            }
        });
        let frame = Frame {
            view_proj: proj * view,
            camera,
            key: (right * -0.5 + up * 0.6 + back * 0.65).normalize(),
            fill: (right * 0.8 + up * -0.1 + back * 0.5).normalize(),
            rim: (right * 0.45 + up * 0.5 + back * -0.75).normalize(),
            iris: IRIS,
            eyes,
            wireframe: self.wireframe,
        };
        let shared = self.shared.clone();
        let callback = egui_glow::CallbackFn::new(move |info, painter| {
            shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .paint(&info, painter, &frame);
        });
        painter.add(egui::PaintCallback {
            rect,
            callback: Arc::new(callback),
        });
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

fn paint_background(painter: &egui::Painter, rect: Rect, dark: bool) {
    let (inner, outer) = if dark {
        (Color32::from_rgb(40, 47, 68), Color32::from_rgb(13, 15, 21))
    } else {
        (
            Color32::from_rgb(246, 247, 251),
            Color32::from_rgb(204, 209, 220),
        )
    };
    const N: u32 = 16;
    let aspect = rect.width() / rect.height().max(1.0);
    let mut mesh = egui::Mesh::default();
    for j in 0..=N {
        for i in 0..=N {
            let u = i as f32 / N as f32;
            let v = j as f32 / N as f32;
            let dx = (u - 0.5) * aspect.min(2.0);
            let dy = v - 0.42;
            let r = (dx * dx + dy * dy).sqrt();
            let t = (r / 0.8).clamp(0.0, 1.0);
            let t = t * t * (3.0 - 2.0 * t);
            let pos: Pos2 = rect.lerp_inside(vec2(u, v));
            mesh.colored_vertex(pos, lerp_color(inner, outer, t));
        }
    }
    for j in 0..N {
        for i in 0..N {
            let a = j * (N + 1) + i;
            let b = a + 1;
            let c = a + N + 1;
            let d = c + 1;
            mesh.add_triangle(a, b, d);
            mesh.add_triangle(a, d, c);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn top_shapes(shapes: &[f32; NUM_SHAPES]) -> Vec<(UnifiedExpressions, f32)> {
    let mut list: Vec<(UnifiedExpressions, f32)> = UnifiedExpressions::iter()
        .map(|e| (e, signed(shapes[e as usize])))
        .filter(|(_, v)| v.abs() > 0.02)
        .collect();
    list.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
    list.truncate(TOP_SHAPES);
    list
}

fn gaze_angles(
    shapes: &[f32; NUM_SHAPES],
    eyes: [Option<Vec3>; 2],
    prefer_shapes: bool,
) -> [Vec2; 2] {
    let from_euler = |e: Vec3| e.is_finite().then(|| gvec2(e.y, e.x));
    let tracked = if prefer_shapes {
        None
    } else {
        eyes[0].and_then(from_euler)
    };
    let pair = match tracked {
        Some(left) => [left, eyes[1].and_then(from_euler).unwrap_or(left)],
        None => {
            let s = |e: UnifiedExpressions| signed(shapes[e as usize]);
            let pitch = s(UnifiedExpressions::EyeY) * GAZE_MAX;
            [
                gvec2(-s(UnifiedExpressions::EyeLeftX) * GAZE_MAX, pitch),
                gvec2(-s(UnifiedExpressions::EyeRightX) * GAZE_MAX, pitch),
            ]
        }
    };
    pair.map(|g| gvec2(g.x.clamp(-0.65, 0.65), g.y.clamp(-0.55, 0.55)))
}

fn lid_angles(closed: f32, wide: f32, squint: f32, cheek: f32, pitch: f32) -> Lid {
    let deg = |d: f32| d.to_radians();
    let upper_open = deg(22.0 + 20.0 * wide - 7.0 * squint) + pitch * 0.45;
    let lower_open = deg(-25.0 + 4.0 * wide + 13.0 * squint + 7.0 * cheek) + pitch * 0.25;
    let meet = deg(-6.0 + 3.0 * squint) + pitch * 0.3;
    Lid {
        upper: upper_open + (meet - deg(1.0) - upper_open) * closed,
        lower: lower_open + (meet + deg(1.0) - lower_open) * closed,
    }
}

fn pose_from_shapes(shapes: &[f32; NUM_SHAPES], gaze: &[Vec2; 2]) -> Pose {
    use UnifiedExpressions as U;
    let u = |e: U| unit(shapes[e as usize]);
    let mut pose = Pose::default();
    let pairs = [
        (Morph::BrowInnerUp, U::BrowInnerUpLeft, U::BrowInnerUpRight),
        (Morph::BrowOuterUp, U::BrowOuterUpLeft, U::BrowOuterUpRight),
        (Morph::BrowLowerer, U::BrowLowererLeft, U::BrowLowererRight),
        (Morph::BrowPinch, U::BrowPinchLeft, U::BrowPinchRight),
        (Morph::EyeSquint, U::EyeSquintLeft, U::EyeSquintRight),
        (Morph::EyeWide, U::EyeWideLeft, U::EyeWideRight),
        (
            Morph::NasalDilation,
            U::NasalDilationLeft,
            U::NasalDilationRight,
        ),
        (
            Morph::NasalConstrict,
            U::NasalConstrictLeft,
            U::NasalConstrictRight,
        ),
        (Morph::NoseSneer, U::NoseSneerLeft, U::NoseSneerRight),
        (Morph::CheekSquint, U::CheekSquintLeft, U::CheekSquintRight),
        (Morph::CheekPuff, U::CheekPuffLeft, U::CheekPuffRight),
        (Morph::CheekSuck, U::CheekSuckLeft, U::CheekSuckRight),
        (Morph::JawClench, U::JawClench, U::JawClench),
        (
            Morph::LipSuckUpper,
            U::LipSuckUpperLeft,
            U::LipSuckUpperRight,
        ),
        (
            Morph::LipSuckLower,
            U::LipSuckLowerLeft,
            U::LipSuckLowerRight,
        ),
        (
            Morph::LipSuckCorner,
            U::LipSuckCornerLeft,
            U::LipSuckCornerRight,
        ),
        (
            Morph::LipFunnelUpper,
            U::LipFunnelUpperLeft,
            U::LipFunnelUpperRight,
        ),
        (
            Morph::LipFunnelLower,
            U::LipFunnelLowerLeft,
            U::LipFunnelLowerRight,
        ),
        (
            Morph::LipPuckerUpper,
            U::LipPuckerUpperLeft,
            U::LipPuckerUpperRight,
        ),
        (
            Morph::LipPuckerLower,
            U::LipPuckerLowerLeft,
            U::LipPuckerLowerRight,
        ),
        (
            Morph::MouthUpperUp,
            U::MouthUpperUpLeft,
            U::MouthUpperUpRight,
        ),
        (
            Morph::MouthUpperDeepen,
            U::MouthUpperDeepenLeft,
            U::MouthUpperDeepenRight,
        ),
        (
            Morph::MouthLowerDown,
            U::MouthLowerDownLeft,
            U::MouthLowerDownRight,
        ),
        (
            Morph::MouthUpperShift,
            U::MouthUpperLeft,
            U::MouthUpperRight,
        ),
        (
            Morph::MouthLowerShift,
            U::MouthLowerLeft,
            U::MouthLowerRight,
        ),
        (
            Morph::MouthCornerPull,
            U::MouthCornerPullLeft,
            U::MouthCornerPullRight,
        ),
        (
            Morph::MouthCornerSlant,
            U::MouthCornerSlantLeft,
            U::MouthCornerSlantRight,
        ),
        (Morph::MouthFrown, U::MouthFrownLeft, U::MouthFrownRight),
        (
            Morph::MouthStretch,
            U::MouthStretchLeft,
            U::MouthStretchRight,
        ),
        (Morph::MouthDimple, U::MouthDimpleLeft, U::MouthDimpleRight),
        (Morph::MouthPress, U::MouthPressLeft, U::MouthPressRight),
        (
            Morph::MouthTightener,
            U::MouthTightenerLeft,
            U::MouthTightenerRight,
        ),
    ];
    for (morph, left, right) in pairs {
        pose.set(morph, Side::Left, u(left));
        pose.set(morph, Side::Right, u(right));
    }
    pose.set(
        Morph::MouthRaiserUpper,
        Side::Center,
        u(U::MouthRaiserUpper),
    );
    pose.set(
        Morph::MouthRaiserLower,
        Side::Center,
        u(U::MouthRaiserLower),
    );

    let tongue_out = u(U::TongueOut);
    pose.jaw = Jaw {
        open: u(U::JawOpen).max(0.45 * tongue_out),
        side: u(U::JawLeft) - u(U::JawRight),
        forward: u(U::JawForward) - u(U::JawBackward),
        raise: u(U::JawMandibleRaise),
        seal: u(U::MouthClosed),
    };
    pose.lids = [
        lid_angles(
            u(U::EyeClosedLeft),
            u(U::EyeWideLeft),
            u(U::EyeSquintLeft),
            u(U::CheekSquintLeft),
            gaze[0].y,
        ),
        lid_angles(
            u(U::EyeClosedRight),
            u(U::EyeWideRight),
            u(U::EyeSquintRight),
            u(U::CheekSquintRight),
            gaze[1].y,
        ),
    ];
    pose.tongue = Tongue {
        out: tongue_out,
        up: u(U::TongueUp),
        down: u(U::TongueDown),
        left: u(U::TongueLeft),
        right: u(U::TongueRight),
        roll: u(U::TongueRoll),
        bend_down: u(U::TongueBendDown),
        curl_up: u(U::TongueCurlUp),
        squish: u(U::TongueSquish),
        flat: u(U::TongueFlat),
        twist: u(U::TongueTwistLeft) - u(U::TongueTwistRight),
    };
    pose.sanitize();
    pose
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::EnumCount;

    fn shapes_with(values: &[(UnifiedExpressions, f32)]) -> [f32; NUM_SHAPES] {
        let mut shapes = [0.0; NUM_SHAPES];
        for (e, v) in values {
            shapes[*e as usize] = *v;
        }
        shapes
    }

    #[test]
    fn garbage_input_yields_a_clean_pose() {
        let mut shapes = [f32::NAN; NUM_SHAPES];
        for (i, s) in shapes.iter_mut().enumerate() {
            *s = match i % 4 {
                0 => f32::NAN,
                1 => f32::INFINITY,
                2 => -50.0,
                _ => 50.0,
            };
        }
        let eyes = [
            Some(Vec3::new(f32::NAN, 1.0, 0.0)),
            Some(Vec3::splat(f32::INFINITY)),
        ];
        let gaze = gaze_angles(&shapes, eyes, false);
        assert!(gaze.iter().all(|g| g.is_finite()));
        let pose = pose_from_shapes(&shapes, &gaze);
        assert!(pose.weights.iter().all(|w| (0.0..=1.0).contains(w)));
        for lid in pose.lids {
            assert!(lid.upper.is_finite() && lid.lower.is_finite());
            assert!(lid.upper >= lid.lower);
        }
        let mut clean = pose;
        clean.sanitize();
        assert_eq!(clean, pose);
    }

    #[test]
    fn shapes_drive_their_targets() {
        let shapes = shapes_with(&[
            (UnifiedExpressions::MouthCornerPullLeft, 0.8),
            (UnifiedExpressions::BrowInnerUpRight, 0.5),
            (UnifiedExpressions::JawOpen, 0.3),
            (UnifiedExpressions::TongueOut, 1.0),
            (UnifiedExpressions::MouthClosed, 0.4),
        ]);
        let gaze = gaze_angles(&shapes, [None, None], false);
        let pose = pose_from_shapes(&shapes, &gaze);
        assert_eq!(pose.weight(Morph::MouthCornerPull, Side::Left), 0.8);
        assert_eq!(pose.weight(Morph::MouthCornerPull, Side::Right), 0.0);
        assert_eq!(pose.weight(Morph::BrowInnerUp, Side::Right), 0.5);
        assert_eq!(pose.tongue.out, 1.0);
        assert!(pose.jaw.open >= 0.45);
        assert_eq!(pose.jaw.seal, 0.4);
    }

    #[test]
    fn closed_eyes_close_the_lids() {
        let shapes = shapes_with(&[(UnifiedExpressions::EyeClosedLeft, 1.0)]);
        let pose = pose_from_shapes(&shapes, &[Vec2::ZERO; 2]);
        let gap = |l: Lid| l.upper - l.lower;
        assert!(gap(pose.lids[0]) <= 0.0);
        assert!(gap(pose.lids[1]) > 0.5);
    }

    #[test]
    fn gaze_uses_tracker_unless_shapes_are_pinned() {
        let shapes = shapes_with(&[
            (UnifiedExpressions::EyeLeftX, 1.0),
            (UnifiedExpressions::EyeRightX, 1.0),
        ]);
        let tracked = Some(Vec3::new(0.2, 0.3, 0.0));
        let g = gaze_angles(&shapes, [tracked, None], false);
        assert_eq!(g[0], gvec2(0.3, 0.2));
        assert_eq!(g[1], g[0]);
        let g = gaze_angles(&shapes, [tracked, None], true);
        assert!(g[0].x < 0.0 && g[1].x < 0.0);
        let g = gaze_angles(&shapes, [None, None], false);
        assert!((g[0].x + GAZE_MAX).abs() < 1e-5);
    }

    #[test]
    fn readout_is_sorted_and_bounded() {
        let mut shapes = [0.0; NUM_SHAPES];
        for (i, s) in shapes
            .iter_mut()
            .enumerate()
            .take(UnifiedExpressions::COUNT)
        {
            *s = (i as f32 * 0.37).sin();
        }
        shapes[0] = f32::NAN;
        let top = top_shapes(&shapes);
        assert!(top.len() <= TOP_SHAPES);
        assert!(top.windows(2).all(|w| w[0].1.abs() >= w[1].1.abs()));
        assert!(top.iter().all(|(_, v)| v.is_finite()));
    }
}
