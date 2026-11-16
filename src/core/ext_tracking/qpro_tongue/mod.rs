use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use colored::{Color, Colorize};
use once_cell::sync::Lazy;

use crate::core::status::StatusBar;

use self::{
    model::{TongueModel, VisibilityGate},
    stream::FrameSlot,
};

use super::unified::{UnifiedExpressions, UnifiedTrackingData};

mod model;
mod stream;

const STALE_AFTER: Duration = Duration::from_millis(300);
const FRAME_WAIT: Duration = Duration::from_millis(500);
const LATCH_RELEASE: f32 = 0.08;
const MIN_RESPONSE: f32 = 0.05;
const SMOOTHING_RANGE: f32 = 0.88;
const DEBUG_INTERVAL: Duration = Duration::from_secs(1);

static STA_TONGUE: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "TONGUE".color(Color::Green)).into());
static STA_TONGUE_OFF: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "TONGUE".color(Color::Red)).into());

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Visibility {
    Camera,
    Native,
    #[default]
    Weighted,
    Agreement,
}

#[derive(Clone, Debug, Default, clap::Args)]
pub struct TongueArgs {
    #[arg(
        long,
        value_name = "ONNX",
        help = "Enable camera tongue tracking with this visibility model"
    )]
    #[cfg_attr(feature = "babble", arg(conflicts_with = "babble"))]
    pub tongue_model: Option<PathBuf>,

    #[arg(
        long,
        value_name = "ONNX",
        requires = "tongue_model",
        help = "Optional second model that supplies the tongue direction"
    )]
    pub tongue_direction_model: Option<PathBuf>,

    #[arg(
        long,
        default_value = "27273",
        help = "Local TCP port of the forwarded headset camera stream"
    )]
    pub tongue_port: u16,

    #[arg(
        long,
        default_value = "55",
        value_parser = clap::value_parser!(u8).range(0..=100),
        help = "Tongue motion smoothing, 0 (responsive) to 100 (smooth)"
    )]
    pub tongue_smoothing: u8,

    #[arg(
        long,
        value_enum,
        default_value = "weighted",
        help = "How camera and headset tongue visibility are combined"
    )]
    pub tongue_visibility: Visibility,

    #[arg(
        long,
        value_parser = parse_threshold,
        help = "Override the model's tongue visibility threshold (greater than 0, up to 1)"
    )]
    pub tongue_threshold: Option<f32>,

    #[arg(
        long,
        help = "Log camera/native tongue scores once per second for calibration"
    )]
    pub tongue_debug: bool,

    #[arg(long, default_value = "2", help = "CPU threads used per tongue model")]
    pub tongue_threads: usize,
}

fn parse_threshold(text: &str) -> Result<f32, String> {
    let value: f32 = text.parse().map_err(|_| "expected a number".to_string())?;
    if value.is_finite() && value > 0.0 && value <= 1.0 {
        Ok(value)
    } else {
        Err("threshold must be greater than 0 and at most 1".into())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Pose {
    visibility: f32,
    extension: f32,
    horizontal: f32,
    vertical: f32,
    curl_up: f32,
    bend_down: f32,
    roll: f32,
    flat: f32,
    squish: f32,
    twist: f32,
}

impl Pose {
    fn from_targets(targets: &[String], values: &[f32]) -> Self {
        let get = |name: &str| {
            targets
                .iter()
                .position(|target| target == name)
                .map_or(0.0, |index| values[index])
        };
        Self {
            visibility: get("visibility"),
            extension: get("extension"),
            horizontal: get("horizontal"),
            vertical: get("vertical"),
            curl_up: get("curl_up"),
            bend_down: get("bend_down"),
            roll: get("roll"),
            flat: get("flat"),
            squish: get("squish"),
            twist: get("twist"),
        }
    }

    fn shapes(&self) -> [(UnifiedExpressions, f32); 12] {
        let unit = |value: f32| value.clamp(0.0, 1.0);
        let horizontal = self.horizontal.clamp(-1.0, 1.0);
        let vertical = self.vertical.clamp(-1.0, 1.0);
        let twist = self.twist.clamp(-1.0, 1.0);
        [
            (UnifiedExpressions::TongueOut, unit(self.extension)),
            (UnifiedExpressions::TongueUp, vertical.max(0.0)),
            (UnifiedExpressions::TongueDown, (-vertical).max(0.0)),
            (UnifiedExpressions::TongueLeft, (-horizontal).max(0.0)),
            (UnifiedExpressions::TongueRight, horizontal.max(0.0)),
            (UnifiedExpressions::TongueRoll, unit(self.roll)),
            (UnifiedExpressions::TongueBendDown, unit(self.bend_down)),
            (UnifiedExpressions::TongueCurlUp, unit(self.curl_up)),
            (UnifiedExpressions::TongueSquish, unit(self.squish)),
            (UnifiedExpressions::TongueFlat, unit(self.flat)),
            (UnifiedExpressions::TongueTwistLeft, (-twist).max(0.0)),
            (UnifiedExpressions::TongueTwistRight, twist.max(0.0)),
        ]
    }
}

struct Estimate {
    pose: Pose,
    gate: VisibilityGate,
    at: Instant,
}

type SharedEstimate = Arc<Mutex<Option<Estimate>>>;

fn fuse(mode: Visibility, camera_weight: f32, camera: f32, native: f32) -> f32 {
    match mode {
        Visibility::Camera => camera,
        Visibility::Native => native,
        Visibility::Agreement => camera.min(native),
        Visibility::Weighted => camera_weight * camera + (1.0 - camera_weight) * native,
    }
}

fn latch(visible: bool, fused: f32, threshold: f32) -> bool {
    if visible {
        fused >= threshold - LATCH_RELEASE
    } else {
        fused >= threshold
    }
}

fn response(smoothing: u8) -> f32 {
    (1.0 - SMOOTHING_RANGE * (smoothing as f32 / 100.0)).clamp(MIN_RESPONSE, 1.0)
}

struct Worker {
    gate_path: PathBuf,
    direction_path: Option<PathBuf>,
    threads: usize,
    response: f32,
    slot: Arc<FrameSlot>,
    estimate: SharedEstimate,
}

impl Worker {
    fn run(&self) -> anyhow::Result<()> {
        let environment = ort::init().with_name("oscavmgr").build()?;
        let mut gate_model = TongueModel::load(&environment, &self.gate_path, self.threads)?;
        let mut direction_model = self
            .direction_path
            .as_ref()
            .map(|path| TongueModel::load(&environment, path, self.threads))
            .transpose()?;

        let visibility_index = gate_model
            .targets
            .iter()
            .position(|target| target == "visibility")
            .ok_or_else(|| anyhow::anyhow!("visibility model has no visibility output"))?;
        if let Some(direction) = direction_model.as_ref() {
            if direction.targets != gate_model.targets {
                anyhow::bail!("visibility and direction models use different outputs");
            }
        }

        let gate = gate_model.gate;
        log::info!(
            "Tongue: models ready (camera weight {:.2}, threshold {:.2})",
            gate.camera_weight,
            gate.threshold
        );

        let mut smoothed: Option<Vec<f32>> = None;
        loop {
            let Some(frame) = self.slot.take(FRAME_WAIT) else {
                if !self.slot.connected() {
                    smoothed = None;
                }
                continue;
            };

            let mut values = gate_model.infer(&frame)?;
            if let Some(direction) = direction_model.as_mut() {
                let gate_visibility = values[visibility_index];
                values = direction.infer(&frame)?;
                values[visibility_index] = gate_visibility;
            }

            if let Some(previous) = smoothed.as_ref() {
                for (value, previous) in values.iter_mut().zip(previous) {
                    *value = self.response * *value + (1.0 - self.response) * previous;
                }
            }

            *self.estimate.lock().unwrap() = Some(Estimate {
                pose: Pose::from_targets(&gate_model.targets, &values),
                gate,
                at: Instant::now(),
            });
            smoothed = Some(values);
        }
    }
}

pub struct QproTongue {
    mode: Visibility,
    threshold: Option<f32>,
    debug: bool,
    last_debug: Option<Instant>,
    estimate: SharedEstimate,
    native: f32,
    visible: bool,
}

impl QproTongue {
    pub fn new(args: TongueArgs) -> Option<Self> {
        let estimate = SharedEstimate::default();
        let worker = Worker {
            gate_path: args.tongue_model?,
            direction_path: args.tongue_direction_model,
            threads: args.tongue_threads.max(1),
            response: response(args.tongue_smoothing),
            slot: stream::spawn(args.tongue_port),
            estimate: estimate.clone(),
        };

        thread::Builder::new()
            .name("tongue-inference".into())
            .spawn(move || {
                if let Err(e) = worker.run() {
                    log::error!("Tongue: {e:#}");
                }
                *worker.estimate.lock().unwrap() = None;
            })
            .expect("spawn tongue-inference thread");

        Some(Self {
            mode: args.tongue_visibility,
            threshold: args.tongue_threshold,
            debug: args.tongue_debug,
            last_debug: None,
            estimate,
            native: 0.0,
            visible: false,
        })
    }

    pub fn apply(
        &mut self,
        native: Option<f32>,
        data: &mut UnifiedTrackingData,
        status: &mut StatusBar,
    ) {
        if let Some(native) = native {
            self.native = native.clamp(0.0, 1.0);
        }

        let fresh = self
            .estimate
            .lock()
            .unwrap()
            .as_ref()
            .filter(|estimate| estimate.at.elapsed() <= STALE_AFTER)
            .map(|estimate| (estimate.pose, estimate.gate));
        let Some((pose, gate)) = fresh else {
            self.visible = false;
            status.add_item(STA_TONGUE_OFF.clone());
            return;
        };

        let fused = fuse(self.mode, gate.camera_weight, pose.visibility, self.native);
        let threshold = self.threshold.unwrap_or(gate.threshold);
        self.visible = latch(self.visible, fused, threshold);

        if self.debug
            && self
                .last_debug
                .is_none_or(|at| at.elapsed() >= DEBUG_INTERVAL)
        {
            log::info!(
                "Tongue scores: camera={:.3} native={:.3} native_fresh={} mode={:?} fused={:.3} threshold={:.3} extension={:.3} visible={}",
                pose.visibility, self.native, native.is_some(), self.mode, fused, threshold,
                pose.extension, self.visible
            );
            self.last_debug = Some(Instant::now());
        }

        for (expression, value) in pose.shapes() {
            data.setu(expression, if self.visible { value } else { 0.0 });
        }
        status.add_item(STA_TONGUE.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(shapes: &[(UnifiedExpressions, f32); 12], expression: UnifiedExpressions) -> f32 {
        shapes
            .iter()
            .find(|(e, _)| *e as usize == expression as usize)
            .unwrap()
            .1
    }

    #[test]
    fn signed_outputs_split_into_opposite_shapes() {
        let pose = Pose {
            horizontal: -0.6,
            vertical: 0.4,
            twist: 0.25,
            ..Default::default()
        };
        let shapes = pose.shapes();
        assert_eq!(value(&shapes, UnifiedExpressions::TongueLeft), 0.6);
        assert_eq!(value(&shapes, UnifiedExpressions::TongueRight), 0.0);
        assert_eq!(value(&shapes, UnifiedExpressions::TongueUp), 0.4);
        assert_eq!(value(&shapes, UnifiedExpressions::TongueDown), 0.0);
        assert_eq!(value(&shapes, UnifiedExpressions::TongueTwistRight), 0.25);
        assert_eq!(value(&shapes, UnifiedExpressions::TongueTwistLeft), 0.0);
    }

    #[test]
    fn confident_tip_detection_preserves_partial_extension() {
        let pose = Pose {
            visibility: 0.99,
            extension: 0.3,
            ..Default::default()
        };
        assert_eq!(value(&pose.shapes(), UnifiedExpressions::TongueOut), 0.3);
    }

    #[test]
    fn fusion_modes_match_upstream() {
        assert_eq!(fuse(Visibility::Camera, 0.95, 0.8, 0.2), 0.8);
        assert_eq!(fuse(Visibility::Native, 0.95, 0.8, 0.2), 0.2);
        assert_eq!(fuse(Visibility::Agreement, 0.95, 0.8, 0.2), 0.2);
        assert!((fuse(Visibility::Weighted, 0.95, 0.8, 0.2) - 0.77).abs() < 1e-6);
    }

    #[test]
    fn latch_releases_below_the_entry_threshold() {
        assert!(!latch(false, 0.84, 0.85));
        assert!(latch(false, 0.85, 0.85));
        assert!(latch(true, 0.78, 0.85));
        assert!(!latch(true, 0.76, 0.85));
    }

    #[test]
    fn smoothing_maps_to_response() {
        assert_eq!(response(0), 1.0);
        assert!((response(55) - 0.516).abs() < 1e-6);
        assert!((response(100) - 0.12).abs() < 1e-6);
    }

    #[test]
    fn threshold_requires_a_finite_positive_unit_value() {
        assert_eq!(parse_threshold("0.7"), Ok(0.7));
        assert_eq!(parse_threshold("1"), Ok(1.0));
        for invalid in ["0", "-0.1", "1.1", "NaN", "inf", "text"] {
            assert!(parse_threshold(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn agreement_blocks_a_confident_camera_when_native_is_zero() {
        let fused = fuse(Visibility::Agreement, 0.5, 0.99, 0.0);
        assert!(!latch(false, fused, 0.44));
        let camera = fuse(Visibility::Camera, 0.5, 0.99, 0.0);
        assert!(latch(false, camera, 0.7));
    }

    #[test]
    fn pose_reads_targets_by_name() {
        let targets: Vec<String> = ["visibility", "horizontal", "roll"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let pose = Pose::from_targets(&targets, &[0.9, -0.5, 0.2]);
        assert_eq!(pose.visibility, 0.9);
        assert_eq!(pose.horizontal, -0.5);
        assert_eq!(pose.roll, 0.2);
        assert_eq!(pose.vertical, 0.0);
    }
}
