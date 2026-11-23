use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{Duration, Instant},
};

use colored::{Color, Colorize};
use once_cell::sync::Lazy;

use crate::core::{status::StatusBar, AppState};

use self::{babble_face::MouthMapping, calib::Extent, etvr_eyes::GazeLimits};

use super::{unified::UnifiedTrackingData, FaceReceiver};

pub mod babble_face;
pub mod calib;
pub mod config;
mod decode;
pub mod etvr_eyes;
pub mod onefilter;
pub mod process;
mod source;
mod worker;

pub use self::{
    babble_face::OUTPUT_COUNT as MOUTH_OUTPUTS,
    calib::Ranges,
    config::CameraConfig,
    etvr_eyes::{EyeState, EXPRESSION_NAMES, OUTPUT_COUNT as EYE_OUTPUTS},
};

pub const STALE_AFTER: Duration = Duration::from_secs(1);
const PREVIEW_HOLD: Duration = Duration::from_millis(1000);

static STA_MOUTH: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "BABBLE-CAM".color(Color::Green)).into());
static STA_MOUTH_OFF: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "BABBLE-CAM".color(Color::Red)).into());
static STA_EYES: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "ETVR-CAM".color(Color::Green)).into());
static STA_EYES_OFF: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "ETVR-CAM".color(Color::Red)).into());

#[derive(Clone, Debug, Default, clap::Args)]
pub struct CameraArgs {
    #[arg(
        long,
        value_name = "SOURCE",
        help = "Read a Babble mouth camera directly: serial port (/dev/ttyUSB0), MJPEG URL (http://babble.local/) or V4L2 device (/dev/video0)"
    )]
    pub babble_camera: Option<String>,

    #[arg(
        long,
        value_name = "ONNX",
        help = "Babble face model (faceModel.onnx from Baballonia or model.onnx from ProjectBabble)"
    )]
    pub babble_model: Option<PathBuf>,

    #[arg(
        long,
        value_name = "SOURCE",
        help = "Read the left EyeTrackVR eye camera directly (serial port, MJPEG URL such as http://etvr-left.local/, or V4L2 device)"
    )]
    pub etvr_left: Option<String>,

    #[arg(
        long,
        value_name = "SOURCE",
        help = "Read the right EyeTrackVR eye camera directly (serial port, MJPEG URL or V4L2 device)"
    )]
    pub etvr_right: Option<String>,

    #[arg(
        long,
        value_name = "ONNX",
        help = "EyeTrackVR NEXT stereo eye model (NEXT_ETVR*.onnx, NEXT_BSB*.onnx or NEXT_PSVR*.onnx)"
    )]
    pub etvr_model: Option<PathBuf>,

    #[arg(long, default_value = "2", help = "CPU threads used per camera model")]
    pub camera_threads: usize,
}

impl CameraArgs {
    pub(super) fn from_setup(setup: &crate::FaceSetup) -> Self {
        match setup {
            #[cfg(feature = "openxr")]
            crate::FaceSetup::Openxr(args) => args.cameras.clone(),
            crate::FaceSetup::Babble { cameras, .. } => cameras.clone(),
            #[allow(unreachable_patterns)]
            _ => Self::default(),
        }
    }

    fn apply(&self, config: &mut CameraConfig) {
        if let Some(source) = self.babble_camera.clone() {
            config.mouth.source = Some(source);
        }
        if let Some(model) = self.babble_model.clone() {
            config.mouth.model = Some(model);
        }
        if let Some(source) = self.etvr_left.clone() {
            config.eyes.left.source = Some(source);
        }
        if let Some(source) = self.etvr_right.clone() {
            config.eyes.right.source = Some(source);
        }
        if let Some(model) = self.etvr_model.clone() {
            config.eyes.model = Some(model);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CameraId {
    Mouth,
    Left,
    Right,
}

impl CameraId {
    pub const ALL: [CameraId; 3] = [CameraId::Mouth, CameraId::Left, CameraId::Right];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            CameraId::Mouth => "Babble camera",
            CameraId::Left => "ETVR left camera",
            CameraId::Right => "ETVR right camera",
        }
    }

    fn color(self) -> bool {
        self != CameraId::Mouth
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkState {
    #[default]
    Off,
    Connecting,
    Streaming,
    Retrying,
}

#[derive(Clone, Debug, Default)]
pub struct CameraTelemetry {
    pub state: LinkState,
    pub description: String,
    pub error: Option<String>,
    pub fps: f32,
    pub width: usize,
    pub height: usize,
    pub frames: u64,
    pub bad_frames: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ModelTelemetry {
    pub path: Option<PathBuf>,
    pub ready: bool,
    pub error: Option<String>,
    pub fps: f32,
    pub latency_ms: f32,
}

#[derive(Clone, Debug)]
pub struct MouthTelemetry {
    pub model: ModelTelemetry,
    pub raw: [f32; MOUTH_OUTPUTS],
    pub output: [f32; MOUTH_OUTPUTS],
}

impl Default for MouthTelemetry {
    fn default() -> Self {
        Self {
            model: ModelTelemetry::default(),
            raw: [0.0; MOUTH_OUTPUTS],
            output: [0.0; MOUTH_OUTPUTS],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EyesTelemetry {
    pub model: ModelTelemetry,
    pub raw: [f32; EYE_OUTPUTS],
    pub state: EyeState,
    pub present: [bool; 2],
}

#[derive(Clone)]
pub struct Preview {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    pub generation: u64,
}

pub struct Shared {
    pub config: Mutex<CameraConfig>,
    pub cameras: Mutex<[CameraTelemetry; 3]>,
    pub mouth: Mutex<MouthTelemetry>,
    pub eyes: Mutex<EyesTelemetry>,
    pub previews: Mutex<[Option<Preview>; 3]>,
    pub inputs: Mutex<[Option<Preview>; 3]>,
    running: AtomicBool,
    mouth_estimate: Mutex<Option<([f32; MOUTH_OUTPUTS], Instant)>>,
    eyes_estimate: Mutex<Option<(EyeState, GazeLimits, Instant)>>,
    preview_at: AtomicU64,
    epoch: Instant,
    mouth_calibrating: AtomicBool,
    mouth_calibration_fresh: AtomicBool,
    eyes_calibrating: AtomicBool,
    eyes_calibration_fresh: AtomicBool,
    recenter: AtomicBool,
    gaze_extent: Mutex<Option<[Extent; 3]>>,
}

pub static SHARED: Lazy<Shared> = Lazy::new(|| Shared {
    config: Mutex::new(CameraConfig::default()),
    cameras: Mutex::new(Default::default()),
    mouth: Mutex::new(MouthTelemetry::default()),
    eyes: Mutex::new(EyesTelemetry::default()),
    previews: Mutex::new(Default::default()),
    inputs: Mutex::new(Default::default()),
    running: AtomicBool::new(false),
    mouth_estimate: Mutex::new(None),
    eyes_estimate: Mutex::new(None),
    preview_at: AtomicU64::new(0),
    epoch: Instant::now(),
    mouth_calibrating: AtomicBool::new(false),
    mouth_calibration_fresh: AtomicBool::new(false),
    eyes_calibrating: AtomicBool::new(false),
    eyes_calibration_fresh: AtomicBool::new(false),
    recenter: AtomicBool::new(false),
    gaze_extent: Mutex::new(None),
});

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn running() -> bool {
    SHARED.running.load(Ordering::Relaxed)
}

pub fn config() -> CameraConfig {
    lock(&SHARED.config).clone()
}

pub fn update_config(change: impl FnOnce(&mut CameraConfig)) {
    let mut config = lock(&SHARED.config);
    change(&mut config);
    let current = std::mem::take(&mut *config);
    *config = current.sanitized();
}

pub fn save_config() -> anyhow::Result<PathBuf> {
    let config = config();
    config.save()
}

pub fn config_path() -> PathBuf {
    CameraConfig::path()
}

pub fn saved_config() -> Option<CameraConfig> {
    CameraConfig::load().ok().flatten()
}

pub fn request_preview() {
    let now = SHARED.epoch.elapsed().as_millis() as u64;
    SHARED.preview_at.store(now.max(1), Ordering::Relaxed);
}

fn preview_wanted() -> bool {
    let at = SHARED.preview_at.load(Ordering::Relaxed);
    at != 0
        && (SHARED.epoch.elapsed().as_millis() as u64).saturating_sub(at)
            <= PREVIEW_HOLD.as_millis() as u64
}

pub fn mouth_calibrating() -> bool {
    SHARED.mouth_calibrating.load(Ordering::Relaxed)
}

pub fn start_mouth_calibration() {
    SHARED
        .mouth_calibration_fresh
        .store(true, Ordering::Relaxed);
    SHARED.mouth_calibrating.store(true, Ordering::Relaxed);
    log::info!("Babble camera: calibration started; make every mouth shape a few times");
}

pub fn stop_mouth_calibration() -> anyhow::Result<PathBuf> {
    SHARED.mouth_calibrating.store(false, Ordering::Relaxed);
    log::info!("Babble camera: calibration finished");
    save_config()
}

pub fn reset_mouth_calibration() {
    SHARED.mouth_calibrating.store(false, Ordering::Relaxed);
    update_config(|config| config.mouth.calibration = Ranges::identity(MOUTH_OUTPUTS));
}

pub fn eyes_calibrating() -> bool {
    SHARED.eyes_calibrating.load(Ordering::Relaxed)
}

pub fn start_eye_calibration() {
    *lock(&SHARED.gaze_extent) = None;
    SHARED.eyes_calibration_fresh.store(true, Ordering::Relaxed);
    SHARED.eyes_calibrating.store(true, Ordering::Relaxed);
    log::info!(
        "ETVR camera: calibration started; look to every edge, blink, squint and raise your brows"
    );
}

pub fn stop_eye_calibration() -> anyhow::Result<PathBuf> {
    SHARED.eyes_calibrating.store(false, Ordering::Relaxed);
    if let Some([left_x, right_x, y]) = lock(&SHARED.gaze_extent).take() {
        update_config(|config| {
            let gaze = &mut config.eyes.settings.gaze;
            gaze.left_x = left_x.axis(gaze.left_x);
            gaze.right_x = right_x.axis(gaze.right_x);
            gaze.y = y.axis(gaze.y);
        });
    }
    log::info!("ETVR camera: calibration finished");
    save_config()
}

pub fn reset_eye_calibration() {
    SHARED.eyes_calibrating.store(false, Ordering::Relaxed);
    *lock(&SHARED.gaze_extent) = None;
    update_config(|config| {
        config.eyes.settings.gaze = Default::default();
        config.eyes.settings.expressions = Ranges::identity(EXPRESSION_NAMES.len());
    });
}

pub fn recenter_gaze() {
    SHARED.recenter.store(true, Ordering::Relaxed);
}

pub(super) fn wrap(inner: Box<dyn FaceReceiver>, args: CameraArgs) -> Box<dyn FaceReceiver> {
    Box::new(CameraReceiver {
        inner,
        args,
        baseline: UnifiedTrackingData::default(),
        mapping: MouthMapping::new(),
        mouth_fresh: false,
        eyes_fresh: false,
    })
}

struct CameraReceiver {
    inner: Box<dyn FaceReceiver>,
    args: CameraArgs,
    baseline: UnifiedTrackingData,
    mapping: MouthMapping,
    mouth_fresh: bool,
    eyes_fresh: bool,
}

impl CameraReceiver {
    fn merge(&mut self, data: &mut UnifiedTrackingData, status: &mut StatusBar) {
        let (mouth_enabled, eyes_enabled) = {
            let config = lock(&SHARED.config);
            (config.mouth_enabled(), config.eyes_enabled())
        };
        let mouth = merge_mouth(&self.mapping, data);
        let eyes = merge_eyes(data);
        if mouth != self.mouth_fresh {
            if mouth {
                log::info!("Babble camera: mouth tracking active");
            } else if mouth_enabled {
                log::warn!("Babble camera: no fresh mouth data; falling back to the provider");
            }
            self.mouth_fresh = mouth;
        }
        if eyes != self.eyes_fresh {
            if eyes {
                log::info!("ETVR camera: eye tracking active");
            } else if eyes_enabled {
                log::warn!("ETVR camera: no fresh eye data; falling back to the provider");
            }
            self.eyes_fresh = eyes;
        }
        if mouth_enabled {
            status.add_item(if mouth {
                STA_MOUTH.clone()
            } else {
                STA_MOUTH_OFF.clone()
            });
        }
        if eyes_enabled {
            status.add_item(if eyes {
                STA_EYES.clone()
            } else {
                STA_EYES_OFF.clone()
            });
        }
    }
}

fn merge_mouth(mapping: &MouthMapping, data: &mut UnifiedTrackingData) -> bool {
    let fresh = lock(&SHARED.mouth_estimate)
        .as_ref()
        .filter(|(_, at)| at.elapsed() <= STALE_AFTER)
        .map(|(values, _)| *values);
    match fresh {
        Some(values) => {
            mapping.apply(&values, data);
            true
        }
        None => false,
    }
}

fn merge_eyes(data: &mut UnifiedTrackingData) -> bool {
    let fresh = lock(&SHARED.eyes_estimate)
        .as_ref()
        .filter(|(_, _, at)| at.elapsed() <= STALE_AFTER)
        .map(|(state, limits, _)| (*state, *limits));
    match fresh {
        Some((state, limits)) => {
            etvr_eyes::apply(&state, &limits, data);
            true
        }
        None => false,
    }
}

impl FaceReceiver for CameraReceiver {
    fn start_loop(&mut self) {
        self.inner.start_loop();
        worker::start(&self.args);
    }

    fn receive(&mut self, data: &mut UnifiedTrackingData, app: &mut AppState) {
        self.inner.receive(&mut self.baseline, app);
        data.eyes = self.baseline.eyes;
        data.shapes.copy_from_slice(&self.baseline.shapes);
        self.merge(data, &mut app.status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ext_tracking::unified::UnifiedExpressions as U;

    #[test]
    fn fresh_camera_estimates_override_and_stale_ones_fall_back() {
        let mapping = MouthMapping::new();
        let mut native = UnifiedTrackingData::default();
        native.setu(U::JawOpen, 0.2);
        native.setu(U::EyeClosedLeft, 0.3);
        native.eyes = [Some(glam::vec3(0.1, 0.0, 0.0)), None];

        let mut values = [0f32; MOUTH_OUTPUTS];
        values[babble_face::OUTPUT_NAMES
            .iter()
            .position(|n| *n == "jawOpen")
            .unwrap()] = 0.9;
        *lock(&SHARED.mouth_estimate) = Some((values, Instant::now()));
        let mut data = native.clone();
        assert!(merge_mouth(&mapping, &mut data));
        assert_eq!(data.getu(U::JawOpen), 0.9);
        assert_eq!(data.getu(U::EyeClosedLeft), 0.3);
        assert_eq!(data.eyes, native.eyes);

        *lock(&SHARED.mouth_estimate) = Some((values, Instant::now() - STALE_AFTER * 2));
        let mut data = native.clone();
        assert!(!merge_mouth(&mapping, &mut data));
        assert_eq!(data.shapes, native.shapes);

        let state = EyeState {
            closed: [1.0, 0.0],
            ..Default::default()
        };
        *lock(&SHARED.eyes_estimate) = Some((state, GazeLimits::default(), Instant::now()));
        let mut data = native.clone();
        assert!(merge_eyes(&mut data));
        assert_eq!(data.getu(U::EyeClosedLeft), 1.0);
        assert_eq!(data.getu(U::JawOpen), 0.2);
        assert_eq!(data.eyes[1], Some(glam::vec3(0.0, 0.0, 0.0)));
        *lock(&SHARED.eyes_estimate) = None;
        *lock(&SHARED.mouth_estimate) = None;
    }

    #[test]
    fn cli_sources_override_the_saved_config() {
        let args = CameraArgs {
            babble_camera: Some("/dev/ttyUSB1".into()),
            etvr_model: Some("eyes.onnx".into()),
            ..Default::default()
        };
        let mut config = CameraConfig::default();
        config.mouth.source = Some("http://old/".into());
        config.eyes.left.source = Some("http://etvr-left.local/".into());
        args.apply(&mut config);
        assert_eq!(config.mouth.source.as_deref(), Some("/dev/ttyUSB1"));
        assert_eq!(
            config.eyes.left.source.as_deref(),
            Some("http://etvr-left.local/")
        );
        assert_eq!(config.eyes.model, Some(PathBuf::from("eyes.onnx")));
    }
}
