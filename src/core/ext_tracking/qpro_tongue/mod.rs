use std::{
    panic::{self, AssertUnwindSafe},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use colored::{Color, Colorize};
use once_cell::sync::{Lazy, OnceCell};
use serde::{Deserialize, Serialize};

use crate::core::{folders::config_file, status::StatusBar};

use self::{
    model::{TongueModel, VisibilityGate},
    stream::{FrameSlot, PANEL},
};

use super::unified::{UnifiedExpressions, UnifiedTrackingData};

mod model;
pub mod relay;
mod stream;

const STALE_AFTER: Duration = Duration::from_millis(300);
const FRAME_WAIT: Duration = Duration::from_millis(500);
const LATCH_RELEASE: f32 = 0.08;
const MIN_RESPONSE: f32 = 0.05;
const SMOOTHING_RANGE: f32 = 0.88;
const DEBUG_INTERVAL: Duration = Duration::from_secs(1);
const WORKER_RETRY_MIN: Duration = Duration::from_secs(2);
const WORKER_RETRY_MAX: Duration = Duration::from_secs(30);
const WORKER_HEALTHY_AFTER: Duration = Duration::from_secs(60);
const PREVIEW_INTERVAL: Duration = Duration::from_millis(66);
pub const SHAPE_COUNT: usize = 12;
pub const PREVIEW_SIZE: usize = PANEL / 2;

static STA_TONGUE: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "TONGUE".color(Color::Green)).into());
static STA_TONGUE_OFF: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "TONGUE".color(Color::Red)).into());
static STA_TONGUE_PAUSED: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "TONGUE".color(Color::Yellow)).into());

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

    #[arg(
        long,
        value_name = "SCRIPT",
        requires = "tongue_model",
        help = "headset-stream.sh the GUI runs to start the headset camera relay"
    )]
    pub tongue_relay_script: Option<PathBuf>,

    #[arg(
        long,
        requires = "tongue_relay_script",
        help = "Start the headset camera relay at launch and stop it again on exit"
    )]
    pub tongue_relay_autostart: bool,
}

fn parse_threshold(text: &str) -> Result<f32, String> {
    let value: f32 = text.parse().map_err(|_| "expected a number".to_string())?;
    if value.is_finite() && value > 0.0 && value <= 1.0 {
        Ok(value)
    } else {
        Err("threshold must be greater than 0 and at most 1".into())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tuning {
    pub enabled: bool,
    pub visibility: Visibility,
    pub threshold: Option<f32>,
    pub camera_weight: Option<f32>,
    pub release: f32,
    pub smoothing: u8,
    pub gains: [f32; SHAPE_COUNT],
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            enabled: true,
            visibility: Visibility::Weighted,
            threshold: None,
            camera_weight: None,
            release: LATCH_RELEASE,
            smoothing: 55,
            gains: [1.0; SHAPE_COUNT],
        }
    }
}

impl Tuning {
    fn from_args(args: &TongueArgs) -> Self {
        Self {
            visibility: args.tongue_visibility,
            threshold: args.tongue_threshold,
            smoothing: args.tongue_smoothing,
            ..Default::default()
        }
    }

    pub fn path() -> PathBuf {
        static PATH: Lazy<PathBuf> =
            Lazy::new(|| PathBuf::from(config_file("tongue.json", Some("oscavmgr-tongue.json"))));
        PATH.clone()
    }

    pub fn load_saved() -> anyhow::Result<Option<Self>> {
        let path = Self::path();
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(Self::sanitized(serde_json::from_slice(&bytes)?))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(anyhow::anyhow!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self) -> anyhow::Result<PathBuf> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_vec_pretty(self)?)?;
        Ok(path)
    }

    fn sanitized(mut self) -> Self {
        self.threshold = self
            .threshold
            .filter(|t| t.is_finite())
            .map(|t| t.clamp(0.01, 1.0));
        self.camera_weight = self
            .camera_weight
            .filter(|w| w.is_finite())
            .map(|w| w.clamp(0.0, 1.0));
        self.release = if self.release.is_finite() {
            self.release.clamp(0.0, 0.5)
        } else {
            LATCH_RELEASE
        };
        self.smoothing = self.smoothing.min(100);
        for gain in self.gains.iter_mut() {
            *gain = if gain.is_finite() {
                gain.clamp(0.0, 3.0)
            } else {
                1.0
            };
        }
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct Telemetry {
    pub connected: bool,
    pub ready: bool,
    pub fresh: bool,
    pub error: Option<String>,
    pub model_threshold: f32,
    pub model_camera_weight: f32,
    pub camera: f32,
    pub native: f32,
    pub native_fresh: bool,
    pub fused: f32,
    pub threshold: f32,
    pub camera_weight: f32,
    pub visible: bool,
    pub horizontal: f32,
    pub vertical: f32,
    pub raw: [f32; SHAPE_COUNT],
    pub output: [f32; SHAPE_COUNT],
    pub fps: f32,
    pub latency_ms: f32,
}

pub struct Preview {
    pub left: Vec<u8>,
    pub right: Vec<u8>,
    pub generation: u64,
}

#[derive(Default)]
pub struct Shared {
    pub active: AtomicBool,
    pub preview_wanted: AtomicBool,
    pub tuning: Mutex<Tuning>,
    pub launch: Mutex<Tuning>,
    pub telemetry: Mutex<Telemetry>,
    pub preview: Mutex<Option<Preview>>,
}

pub static SHARED: Lazy<Shared> = Lazy::new(Shared::default);
static ENGINE: OnceCell<SharedEstimate> = OnceCell::new();

pub fn shape_expressions() -> [UnifiedExpressions; SHAPE_COUNT] {
    Pose::default().shapes().map(|(expression, _)| expression)
}

fn half_size(image: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(PREVIEW_SIZE * PREVIEW_SIZE);
    for y in 0..PREVIEW_SIZE {
        let top = &image[2 * y * PANEL..][..PANEL];
        let bottom = &image[(2 * y + 1) * PANEL..][..PANEL];
        for x in 0..PREVIEW_SIZE {
            let sum = top[2 * x] as u16
                + top[2 * x + 1] as u16
                + bottom[2 * x] as u16
                + bottom[2 * x + 1] as u16;
            output.push((sum / 4) as u8);
        }
    }
    output
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

    fn shapes(&self) -> [(UnifiedExpressions, f32); SHAPE_COUNT] {
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

fn latch(visible: bool, fused: f32, threshold: f32, release: f32) -> bool {
    if visible {
        fused >= threshold - release
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
    slot: Arc<FrameSlot>,
    estimate: SharedEstimate,
}

impl Worker {
    fn run(&self) -> anyhow::Result<()> {
        let environment = crate::core::ext_tracking::onnx_environment()?;
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
        {
            let mut telemetry = SHARED.telemetry.lock().unwrap_or_else(|e| e.into_inner());
            telemetry.ready = true;
            telemetry.model_threshold = gate.threshold;
            telemetry.model_camera_weight = gate.camera_weight;
        }

        let mut smoothed: Option<Vec<f32>> = None;
        let mut last_frame: Option<Instant> = None;
        let mut last_preview: Option<Instant> = None;
        let mut generation = 0u64;
        let mut failed_frames = 0u64;
        loop {
            let frame = self.slot.take(FRAME_WAIT);
            let connected = self.slot.connected();
            SHARED
                .telemetry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .connected = connected;
            let Some(frame) = frame else {
                if !connected {
                    smoothed = None;
                    last_frame = None;
                    let mut telemetry = SHARED.telemetry.lock().unwrap_or_else(|e| e.into_inner());
                    telemetry.fps = 0.0;
                }
                continue;
            };

            let started = Instant::now();
            let inferred = gate_model.infer(&frame).and_then(|mut values| {
                if let Some(direction) = direction_model.as_mut() {
                    let gate_visibility = values[visibility_index];
                    values = direction.infer(&frame)?;
                    values[visibility_index] = gate_visibility;
                }
                Ok(values)
            });
            let mut values = match inferred {
                Ok(values) => {
                    if failed_frames > 0 {
                        log::info!(
                            "Tongue: inference recovered after {failed_frames} failed frames"
                        );
                        failed_frames = 0;
                        SHARED
                            .telemetry
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .error = None;
                    }
                    values
                }
                Err(e) => {
                    failed_frames += 1;
                    if failed_frames == 1 || failed_frames % 500 == 0 {
                        log::warn!("Tongue: skipping frame, inference failed ({failed_frames} so far): {e:#}");
                    }
                    SHARED
                        .telemetry
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .error = Some(format!("{e:#}"));
                    continue;
                }
            };
            let latency_ms = started.elapsed().as_secs_f32() * 1000.0;

            if SHARED.preview_wanted.load(Ordering::Relaxed)
                && last_preview.is_none_or(|at| at.elapsed() >= PREVIEW_INTERVAL)
            {
                generation += 1;
                *SHARED.preview.lock().unwrap_or_else(|e| e.into_inner()) = Some(Preview {
                    left: half_size(&frame.left),
                    right: half_size(&frame.right),
                    generation,
                });
                last_preview = Some(Instant::now());
            }

            let response = response(
                SHARED
                    .tuning
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .smoothing,
            );
            if let Some(previous) = smoothed.as_ref() {
                for (value, previous) in values.iter_mut().zip(previous) {
                    *value = response * *value + (1.0 - response) * previous;
                }
            }

            {
                let mut telemetry = SHARED.telemetry.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(at) = last_frame {
                    let interval = at.elapsed().as_secs_f32().max(1e-3);
                    telemetry.fps = if telemetry.fps > 0.0 {
                        0.9 * telemetry.fps + 0.1 / interval
                    } else {
                        1.0 / interval
                    };
                }
                telemetry.latency_ms = if telemetry.latency_ms > 0.0 {
                    0.9 * telemetry.latency_ms + 0.1 * latency_ms
                } else {
                    latency_ms
                };
            }
            last_frame = Some(Instant::now());

            *self.estimate.lock().unwrap_or_else(|e| e.into_inner()) = Some(Estimate {
                pose: Pose::from_targets(&gate_model.targets, &values),
                gate,
                at: Instant::now(),
            });
            smoothed = Some(values);
        }
    }
}

pub struct QproTongue {
    debug: bool,
    last_debug: Option<Instant>,
    estimate: SharedEstimate,
    native: f32,
    visible: bool,
}

impl QproTongue {
    pub fn new(args: TongueArgs) -> Option<Self> {
        let gate_path = args.tongue_model.clone()?;
        let estimate = ENGINE
            .get_or_init(|| Self::start_engine(&args, gate_path))
            .clone();

        Some(Self {
            debug: args.tongue_debug,
            last_debug: None,
            estimate,
            native: 0.0,
            visible: false,
        })
    }

    fn start_engine(args: &TongueArgs, gate_path: PathBuf) -> SharedEstimate {
        let estimate = SharedEstimate::default();
        let launch = Tuning::from_args(args);
        let tuning = match Tuning::load_saved() {
            Ok(Some(saved)) => {
                log::info!(
                    "Tongue: using saved tuning from {} (overrides command-line tongue options)",
                    Tuning::path().display()
                );
                saved
            }
            Ok(None) => launch.clone(),
            Err(e) => {
                log::warn!("Tongue: ignoring saved tuning: {e:#}");
                launch.clone()
            }
        };
        let worker = Worker {
            gate_path,
            direction_path: args.tongue_direction_model.clone(),
            threads: args.tongue_threads.max(1),
            slot: stream::spawn(args.tongue_port),
            estimate: estimate.clone(),
        };
        relay::configure(
            args.tongue_relay_script.as_deref(),
            args.tongue_port,
            args.tongue_relay_autostart,
            worker.slot.clone(),
        );
        *SHARED.launch.lock().unwrap_or_else(|e| e.into_inner()) = launch;
        *SHARED.tuning.lock().unwrap_or_else(|e| e.into_inner()) = tuning;
        SHARED.active.store(true, Ordering::Relaxed);

        thread::Builder::new()
            .name("tongue-inference".into())
            .spawn(move || {
                let mut delay = WORKER_RETRY_MIN;
                loop {
                    let started = Instant::now();
                    let reason = match panic::catch_unwind(AssertUnwindSafe(|| worker.run())) {
                        Ok(Ok(())) => "stopped".to_string(),
                        Ok(Err(e)) => format!("{e:#}"),
                        Err(payload) => format!(
                            "crashed: {}",
                            payload
                                .downcast_ref::<&str>()
                                .map(|s| s.to_string())
                                .or_else(|| payload.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "unknown panic".into())
                        ),
                    };
                    {
                        let mut telemetry =
                            SHARED.telemetry.lock().unwrap_or_else(|e| e.into_inner());
                        telemetry.ready = false;
                        telemetry.error = Some(reason.clone());
                    }
                    *worker.estimate.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    if started.elapsed() > WORKER_HEALTHY_AFTER {
                        delay = WORKER_RETRY_MIN;
                    }
                    log::error!(
                        "Tongue: {reason}; restarting the tongue models in {}s",
                        delay.as_secs()
                    );
                    thread::sleep(delay);
                    delay = (delay * 2).min(WORKER_RETRY_MAX);
                }
            })
            .expect("spawn tongue-inference thread");

        estimate
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
        let tuning = SHARED
            .tuning
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        let fresh = self
            .estimate
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .filter(|estimate| estimate.at.elapsed() <= STALE_AFTER)
            .map(|estimate| (estimate.pose, estimate.gate));

        let mut telemetry = SHARED.telemetry.lock().unwrap_or_else(|e| e.into_inner());
        telemetry.native = self.native;
        telemetry.native_fresh = native.is_some();
        telemetry.fresh = fresh.is_some();

        let Some((pose, gate)) = fresh else {
            self.visible = false;
            telemetry.visible = false;
            telemetry.output = [0.0; SHAPE_COUNT];
            status.add_item(STA_TONGUE_OFF.clone());
            return;
        };

        let camera_weight = tuning.camera_weight.unwrap_or(gate.camera_weight);
        let fused = fuse(
            tuning.visibility,
            camera_weight,
            pose.visibility,
            self.native,
        );
        let threshold = tuning.threshold.unwrap_or(gate.threshold);
        telemetry.camera = pose.visibility;
        telemetry.fused = fused;
        telemetry.threshold = threshold;
        telemetry.camera_weight = camera_weight;
        telemetry.horizontal = pose.horizontal.clamp(-1.0, 1.0);
        telemetry.vertical = pose.vertical.clamp(-1.0, 1.0);

        if !tuning.enabled {
            self.visible = false;
            telemetry.visible = false;
            for (index, (_, value)) in pose.shapes().into_iter().enumerate() {
                telemetry.raw[index] = value;
                telemetry.output[index] = 0.0;
            }
            status.add_item(STA_TONGUE_PAUSED.clone());
            return;
        }

        self.visible = latch(self.visible, fused, threshold, tuning.release);
        telemetry.visible = self.visible;

        if self.debug
            && self
                .last_debug
                .is_none_or(|at| at.elapsed() >= DEBUG_INTERVAL)
        {
            log::info!(
                "Tongue scores: camera={:.3} native={:.3} native_fresh={} mode={:?} fused={:.3} threshold={:.3} extension={:.3} visible={}",
                pose.visibility, self.native, native.is_some(), tuning.visibility, fused, threshold,
                pose.extension, self.visible
            );
            self.last_debug = Some(Instant::now());
        }

        for (index, (expression, value)) in pose.shapes().into_iter().enumerate() {
            let output = if self.visible {
                (value * tuning.gains[index]).clamp(0.0, 1.0)
            } else {
                0.0
            };
            telemetry.raw[index] = value;
            telemetry.output[index] = output;
            data.setu(expression, output);
        }
        status.add_item(STA_TONGUE.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(
        shapes: &[(UnifiedExpressions, f32); SHAPE_COUNT],
        expression: UnifiedExpressions,
    ) -> f32 {
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
        assert!(!latch(false, 0.84, 0.85, LATCH_RELEASE));
        assert!(latch(false, 0.85, 0.85, LATCH_RELEASE));
        assert!(latch(true, 0.78, 0.85, LATCH_RELEASE));
        assert!(!latch(true, 0.76, 0.85, LATCH_RELEASE));
        assert!(latch(true, 0.66, 0.85, 0.2));
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
        assert!(!latch(false, fused, 0.44, LATCH_RELEASE));
        let camera = fuse(Visibility::Camera, 0.5, 0.99, 0.0);
        assert!(latch(false, camera, 0.7, LATCH_RELEASE));
    }

    #[test]
    fn tuning_round_trips_and_sanitizes() {
        let tuning = Tuning {
            threshold: Some(0.575),
            visibility: Visibility::Agreement,
            ..Default::default()
        };
        let text = serde_json::to_string(&tuning).unwrap();
        assert!(text.contains("\"agreement\""));
        let back: Tuning = serde_json::from_str(&text).unwrap();
        assert_eq!(back, tuning);

        let partial: Tuning = serde_json::from_str(r#"{"smoothing": 250, "release": -1}"#).unwrap();
        let partial = partial.sanitized();
        assert_eq!(partial.smoothing, 100);
        assert_eq!(partial.release, 0.0);
        assert!(partial.enabled);
    }

    #[test]
    fn preview_halves_each_axis() {
        let mut image = vec![0u8; PANEL * PANEL];
        image[0] = 40;
        image[1] = 80;
        image[PANEL] = 120;
        image[PANEL + 1] = 160;
        let preview = half_size(&image);
        assert_eq!(preview.len(), PREVIEW_SIZE * PREVIEW_SIZE);
        assert_eq!(preview[0], 100);
        assert_eq!(preview[1], 0);
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
