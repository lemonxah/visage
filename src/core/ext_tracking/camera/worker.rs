use std::{
    panic::{self, AssertUnwindSafe},
    path::PathBuf,
    sync::{atomic::Ordering, Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

use once_cell::sync::OnceCell;

use super::{
    babble_face::{FaceModel, OUTPUT_COUNT as MOUTH_OUTPUTS},
    calib::Extent,
    config::CameraConfig,
    decode::{self, Image},
    etvr_eyes::{self, EyeModel, INPUT_CHANNELS},
    lock,
    onefilter::OneEuroBank,
    preview_wanted,
    process::{self, plane_to_gray, thumbnail},
    source::{self, FrameSource, SourceSpec},
    CameraArgs, CameraId, CameraTelemetry, LinkState, ModelTelemetry, Preview, SHARED,
};

const IDLE_POLL: Duration = Duration::from_millis(500);
const FRAME_WAIT: Duration = Duration::from_millis(500);
const FRAME_MAX_AGE: Duration = Duration::from_millis(500);
const SOURCE_RETRY_MIN: Duration = Duration::from_secs(1);
const SOURCE_RETRY_MAX: Duration = Duration::from_secs(10);
const SOURCE_HEALTHY: Duration = Duration::from_secs(10);
const WORKER_RETRY_MIN: Duration = Duration::from_secs(2);
const WORKER_RETRY_MAX: Duration = Duration::from_secs(30);
const WORKER_HEALTHY_AFTER: Duration = Duration::from_secs(60);
const PREVIEW_INTERVAL: Duration = Duration::from_millis(66);
const PREVIEW_WIDTH: usize = 320;
const BAD_FRAME_LIMIT: u32 = 120;

#[derive(Default)]
struct HubState {
    frames: [Option<(Arc<Image>, Instant)>; 3],
    sequence: [u64; 3],
}

#[derive(Default)]
pub struct Hub {
    state: Mutex<HubState>,
    signal: Condvar,
}

impl Hub {
    fn put(&self, id: CameraId, image: Image) {
        let mut state = lock(&self.state);
        state.frames[id.index()] = Some((Arc::new(image), Instant::now()));
        state.sequence[id.index()] += 1;
        drop(state);
        self.signal.notify_all();
    }

    fn clear(&self, id: CameraId) {
        lock(&self.state).frames[id.index()] = None;
    }

    fn wait(&self, ids: &[CameraId], seen: &mut [u64; 3], timeout: Duration) -> bool {
        let state = lock(&self.state);
        let (state, _) = self
            .signal
            .wait_timeout_while(state, timeout, |s| {
                ids.iter()
                    .all(|id| s.sequence[id.index()] == seen[id.index()])
            })
            .unwrap_or_else(|e| e.into_inner());
        let mut fresh = false;
        for id in ids {
            fresh |= state.sequence[id.index()] != seen[id.index()];
            seen[id.index()] = state.sequence[id.index()];
        }
        fresh
    }

    fn latest(&self, id: CameraId) -> Option<Arc<Image>> {
        lock(&self.state).frames[id.index()]
            .as_ref()
            .filter(|(_, at)| at.elapsed() <= FRAME_MAX_AGE)
            .map(|(image, _)| image.clone())
    }
}

static STARTED: OnceCell<()> = OnceCell::new();

pub fn start(args: &CameraArgs) {
    STARTED.get_or_init(|| {
        let mut config = match CameraConfig::load() {
            Ok(Some(saved)) => {
                log::info!(
                    "Cameras: using settings from {}",
                    CameraConfig::path().display()
                );
                saved
            }
            Ok(None) => CameraConfig::default(),
            Err(e) => {
                log::warn!("Cameras: ignoring saved settings: {e:#}");
                CameraConfig::default()
            }
        };
        args.apply(&mut config);
        let config = config.sanitized();
        if config.mouth_enabled() {
            log::info!(
                "Babble camera: {} with model {}",
                config.mouth.source.as_deref().unwrap_or_default(),
                config
                    .mouth
                    .model
                    .as_ref()
                    .map_or("(none; pass --babble-model)".into(), |p| p
                        .display()
                        .to_string())
            );
        }
        if config.eyes_enabled() {
            log::info!(
                "ETVR camera: left {}, right {}, model {}",
                config.eyes.left.source.as_deref().unwrap_or("(none)"),
                config.eyes.right.source.as_deref().unwrap_or("(none)"),
                config
                    .eyes
                    .model
                    .as_ref()
                    .map_or("(none; pass --etvr-model)".into(), |p| p
                        .display()
                        .to_string())
            );
        }
        *lock(&SHARED.config) = config;
        SHARED.running.store(true, Ordering::Relaxed);

        let hub = Arc::new(Hub::default());
        let threads = args.camera_threads.clamp(1, 16);
        for id in CameraId::ALL {
            let hub = hub.clone();
            supervise(
                source_thread_name(id),
                id.label(),
                |_| {},
                move || run_source(id, &hub),
            );
        }
        let mouth_hub = hub.clone();
        supervise(
            "babble-camera-model",
            "Babble camera model",
            |reason| model_failed(&SHARED.mouth, |t| &mut t.model, reason),
            move || run_mouth(&mouth_hub, threads),
        );
        supervise(
            "etvr-camera-model",
            "ETVR camera model",
            |reason| model_failed(&SHARED.eyes, |t| &mut t.model, reason),
            move || run_eyes(&hub, threads),
        );
    });
}

fn source_thread_name(id: CameraId) -> &'static str {
    match id {
        CameraId::Mouth => "babble-camera",
        CameraId::Left => "etvr-left-camera",
        CameraId::Right => "etvr-right-camera",
    }
}

fn model_failed<T>(telemetry: &Mutex<T>, model: fn(&mut T) -> &mut ModelTelemetry, reason: &str) {
    let mut telemetry = lock(telemetry);
    let model = model(&mut telemetry);
    model.ready = false;
    model.fps = 0.0;
    model.error = Some(reason.to_string());
}

fn panic_reason(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

fn supervise(
    name: &'static str,
    label: &'static str,
    on_failure: fn(&str),
    mut body: impl FnMut() -> anyhow::Result<()> + Send + 'static,
) {
    let spawned = thread::Builder::new().name(name.into()).spawn(move || {
        let mut delay = WORKER_RETRY_MIN;
        let mut last: Option<String> = None;
        loop {
            let started = Instant::now();
            let reason = match panic::catch_unwind(AssertUnwindSafe(&mut body)) {
                Ok(Ok(())) => "stopped".to_string(),
                Ok(Err(e)) => format!("{e:#}"),
                Err(payload) => format!("crashed: {}", panic_reason(payload)),
            };
            on_failure(&reason);
            if started.elapsed() > WORKER_HEALTHY_AFTER {
                delay = WORKER_RETRY_MIN;
            }
            if last.as_deref() != Some(reason.as_str()) {
                log::error!("{label}: {reason}; retrying in {}s", delay.as_secs());
                last = Some(reason);
            } else {
                log::debug!("{label}: still failing; retrying in {}s", delay.as_secs());
            }
            thread::sleep(delay);
            delay = (delay * 2).min(WORKER_RETRY_MAX);
        }
    });
    if let Err(e) = spawned {
        log::error!("{label}: could not start thread: {e}");
    }
}

fn current_source(id: CameraId) -> Option<String> {
    lock(&SHARED.config).source(id).map(str::to_string)
}

fn update_camera(id: CameraId, change: impl FnOnce(&mut CameraTelemetry)) {
    change(&mut lock(&SHARED.cameras)[id.index()]);
}

fn set_link(id: CameraId, state: LinkState, description: &str, error: Option<String>) {
    update_camera(id, |camera| {
        camera.state = state;
        camera.description = description.to_string();
        camera.error = error;
        if state != LinkState::Streaming {
            camera.fps = 0.0;
        }
    });
}

fn wait_or_change(id: CameraId, source: &str, delay: Duration) {
    let until = Instant::now() + delay;
    while Instant::now() < until {
        if current_source(id).as_deref() != Some(source) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn ema(previous: f32, sample: f32) -> f32 {
    if previous > 0.0 && previous.is_finite() {
        0.9 * previous + 0.1 * sample
    } else {
        sample
    }
}

fn run_source(id: CameraId, hub: &Hub) -> anyhow::Result<()> {
    let label = id.label();
    let mut backoff = SOURCE_RETRY_MIN;
    let mut last_error: Option<String> = None;
    let mut idle = false;
    loop {
        let Some(text) = current_source(id) else {
            if !idle {
                set_link(id, LinkState::Off, "", None);
                hub.clear(id);
                lock(&SHARED.previews)[id.index()] = None;
                idle = true;
                last_error = None;
            }
            thread::sleep(IDLE_POLL);
            continue;
        };
        idle = false;
        set_link(id, LinkState::Connecting, &text, None);
        let opened = SourceSpec::parse(&text).and_then(|spec| source::open(&spec));
        let mut camera = match opened {
            Ok(camera) => camera,
            Err(e) => {
                let message = format!("{e:#}");
                if last_error.as_deref() != Some(message.as_str()) {
                    log::warn!("{label}: {message}; retrying");
                    last_error = Some(message.clone());
                }
                set_link(id, LinkState::Retrying, &text, Some(message));
                wait_or_change(id, &text, backoff);
                backoff = (backoff * 2).min(SOURCE_RETRY_MAX);
                continue;
            }
        };
        let description = camera.describe();
        log::info!("{label}: streaming from {description}");
        last_error = None;
        set_link(id, LinkState::Streaming, &description, None);
        let started = Instant::now();
        let result = stream(id, &text, camera.as_mut(), hub);
        drop(camera);
        hub.clear(id);
        if started.elapsed() > SOURCE_HEALTHY {
            backoff = SOURCE_RETRY_MIN;
        }
        match result {
            Ok(()) => {
                log::info!("{label}: source changed");
                backoff = SOURCE_RETRY_MIN;
            }
            Err(e) => {
                let message = format!("{e:#}");
                if last_error.as_deref() != Some(message.as_str()) {
                    log::warn!("{label}: {message}; reconnecting");
                    last_error = Some(message.clone());
                }
                set_link(id, LinkState::Retrying, &text, Some(message));
                wait_or_change(id, &text, backoff);
                backoff = (backoff * 2).min(SOURCE_RETRY_MAX);
            }
        }
    }
}

fn stream(id: CameraId, text: &str, camera: &mut dyn FrameSource, hub: &Hub) -> anyhow::Result<()> {
    let mut last_frame: Option<Instant> = None;
    let mut last_preview: Option<Instant> = None;
    let mut generation = 0u64;
    let mut bad_streak = 0u32;
    let mut warned = false;
    loop {
        if current_source(id).as_deref() != Some(text) {
            return Ok(());
        }
        let raw = camera.next_frame()?;
        let image = match decode::decode(&raw, id.color()) {
            Ok(image) => image,
            Err(e) => {
                bad_streak += 1;
                update_camera(id, |camera| camera.bad_frames += 1);
                if !warned {
                    log::warn!("{}: dropping undecodable frame: {e:#}", id.label());
                    warned = true;
                }
                if bad_streak >= BAD_FRAME_LIMIT {
                    anyhow::bail!("camera keeps sending undecodable frames ({e:#})");
                }
                continue;
            }
        };
        bad_streak = 0;
        let now = Instant::now();
        update_camera(id, |camera| {
            if let Some(at) = last_frame {
                let interval = now.duration_since(at).as_secs_f32().max(1e-3);
                camera.fps = ema(camera.fps, 1.0 / interval);
            }
            camera.width = image.width;
            camera.height = image.height;
            camera.frames += 1;
        });
        last_frame = Some(now);
        if preview_wanted() && last_preview.is_none_or(|at| at.elapsed() >= PREVIEW_INTERVAL) {
            let (width, height, pixels) = thumbnail(&image, PREVIEW_WIDTH);
            generation += 1;
            lock(&SHARED.previews)[id.index()] = Some(Preview {
                width,
                height,
                pixels,
                generation,
            });
            last_preview = Some(now);
        }
        hub.put(id, image);
    }
}

struct Pacing {
    last_frame: Option<Instant>,
    last_preview: Option<Instant>,
    generation: u64,
    failures: u64,
}

impl Pacing {
    fn new() -> Self {
        Self {
            last_frame: None,
            last_preview: None,
            generation: 0,
            failures: 0,
        }
    }

    fn preview_due(&mut self) -> Option<u64> {
        if preview_wanted()
            && self
                .last_preview
                .is_none_or(|at| at.elapsed() >= PREVIEW_INTERVAL)
        {
            self.last_preview = Some(Instant::now());
            self.generation += 1;
            Some(self.generation)
        } else {
            None
        }
    }

    fn record(&mut self, model: &mut ModelTelemetry, started: Instant) {
        let now = Instant::now();
        if let Some(at) = self.last_frame {
            let interval = now.duration_since(at).as_secs_f32().max(1e-3);
            model.fps = ema(model.fps, 1.0 / interval);
        }
        model.latency_ms = ema(model.latency_ms, started.elapsed().as_secs_f32() * 1000.0);
        self.last_frame = Some(now);
    }

    fn failed(&mut self, label: &str, error: &anyhow::Error) -> String {
        self.failures += 1;
        if self.failures == 1 || self.failures % 500 == 0 {
            log::warn!(
                "{label}: skipping frame, inference failed ({} so far): {error:#}",
                self.failures
            );
        }
        format!("{error:#}")
    }

    fn succeeded(&mut self, label: &str) -> bool {
        if self.failures > 0 {
            log::info!(
                "{label}: inference recovered after {} failed frames",
                self.failures
            );
            self.failures = 0;
            return true;
        }
        false
    }
}

fn model_idle(model: &mut ModelTelemetry, enabled: bool, path: Option<PathBuf>, missing: &str) {
    model.ready = false;
    model.fps = 0.0;
    model.path = path;
    model.error = (enabled && model.path.is_none()).then(|| missing.to_string());
}

fn environment() -> anyhow::Result<ort::environment::Environment> {
    super::super::onnx_environment()
}

fn run_mouth(hub: &Hub, threads: usize) -> anyhow::Result<()> {
    const LABEL: &str = "Babble camera";
    let mut seen = [0u64; 3];
    loop {
        let (enabled, path) = {
            let config = lock(&SHARED.config);
            (config.mouth_enabled(), config.mouth.model.clone())
        };
        let Some(path) = path.clone().filter(|_| enabled) else {
            model_idle(
                &mut lock(&SHARED.mouth).model,
                enabled,
                path,
                "no Babble model: pass --babble-model or set it in the Cameras tab",
            );
            *lock(&SHARED.mouth_estimate) = None;
            thread::sleep(IDLE_POLL);
            continue;
        };
        {
            let mut telemetry = lock(&SHARED.mouth);
            telemetry.model.path = Some(path.clone());
            telemetry.model.error = None;
        }
        let environment = environment()?;
        let mut model = FaceModel::load(&environment, &path, threads)?;
        lock(&SHARED.mouth).model.ready = true;

        let mut filters = OneEuroBank::new(MOUTH_OUTPUTS);
        let mut pacing = Pacing::new();
        loop {
            let fresh = hub.wait(&[CameraId::Mouth], &mut seen, FRAME_WAIT);
            let mouth = {
                let config = lock(&SHARED.config);
                (config.mouth_enabled() && config.mouth.model.as_ref() == Some(&path))
                    .then(|| config.mouth.clone())
            };
            let Some(mouth) = mouth else {
                *lock(&SHARED.mouth_estimate) = None;
                break;
            };
            if !fresh {
                continue;
            }
            let Some(image) = hub.latest(CameraId::Mouth) else {
                continue;
            };
            let started = Instant::now();
            let mut input = Vec::with_capacity(model.channels * model.size * model.size);
            process::prepare(&image, &mouth.view, model.size, model.channels, &mut input);
            if let Some(generation) = pacing.preview_due() {
                lock(&SHARED.inputs)[CameraId::Mouth.index()] = Some(Preview {
                    width: model.size,
                    height: model.size,
                    pixels: plane_to_gray(&input, model.size, 1.0),
                    generation,
                });
            }
            for value in input.iter_mut() {
                *value /= 255.0;
            }
            let raw = match model.infer(input) {
                Ok(raw) => raw,
                Err(e) => {
                    let message = pacing.failed(LABEL, &e);
                    lock(&SHARED.mouth).model.error = Some(message);
                    continue;
                }
            };
            if pacing.succeeded(LABEL) {
                lock(&SHARED.mouth).model.error = None;
            }
            let now = Instant::now();
            let mut values = raw;
            filters.filter(&mut values, now, &mouth.filter);
            let calibration = if SHARED.mouth_calibrating.load(Ordering::Relaxed) {
                let mut config = lock(&SHARED.config);
                let ranges = &mut config.mouth.calibration;
                if SHARED
                    .mouth_calibration_fresh
                    .swap(false, Ordering::Relaxed)
                {
                    ranges.begin(&values);
                } else {
                    ranges.observe(&values);
                }
                ranges.clone()
            } else {
                mouth.calibration
            };
            calibration.apply(&mut values);
            *lock(&SHARED.mouth_estimate) = Some((values, now));
            let mut telemetry = lock(&SHARED.mouth);
            telemetry.raw = raw;
            telemetry.output = values;
            pacing.record(&mut telemetry.model, started);
        }
    }
}

fn run_eyes(hub: &Hub, threads: usize) -> anyhow::Result<()> {
    const LABEL: &str = "ETVR camera";
    let mut seen = [0u64; 3];
    loop {
        let (enabled, path) = {
            let config = lock(&SHARED.config);
            (config.eyes_enabled(), config.eyes.model.clone())
        };
        let Some(path) = path.clone().filter(|_| enabled) else {
            model_idle(
                &mut lock(&SHARED.eyes).model,
                enabled,
                path,
                "no EyeTrackVR model: pass --etvr-model or set it in the Cameras tab",
            );
            *lock(&SHARED.eyes_estimate) = None;
            thread::sleep(IDLE_POLL);
            continue;
        };
        {
            let mut telemetry = lock(&SHARED.eyes);
            telemetry.model.path = Some(path.clone());
            telemetry.model.error = None;
        }
        let environment = environment()?;
        let mut model = EyeModel::load(&environment, &path, threads)?;
        lock(&SHARED.eyes).model.ready = true;

        let mut filters = OneEuroBank::new(etvr_eyes::OUTPUT_COUNT);
        let mut pacing = Pacing::new();
        loop {
            let fresh = hub.wait(&[CameraId::Left, CameraId::Right], &mut seen, FRAME_WAIT);
            let eyes = {
                let config = lock(&SHARED.config);
                (config.eyes_enabled() && config.eyes.model.as_ref() == Some(&path))
                    .then(|| config.eyes.clone())
            };
            let Some(eyes) = eyes else {
                *lock(&SHARED.eyes_estimate) = None;
                break;
            };
            if !fresh {
                continue;
            }
            let left = hub.latest(CameraId::Left);
            let right = hub.latest(CameraId::Right);
            if left.is_none() && right.is_none() {
                continue;
            }
            let started = Instant::now();
            let size = model.size;
            let plane = 3 * size * size;
            let mut input = Vec::with_capacity(INPUT_CHANNELS * size * size);
            for (image, view) in [(&left, &eyes.left.view), (&right, &eyes.right.view)] {
                match image {
                    Some(image) => process::prepare(image, view, size, 3, &mut input),
                    None => input.resize(input.len() + plane, 0.0),
                }
            }
            if let Some(generation) = pacing.preview_due() {
                let mut inputs = lock(&SHARED.inputs);
                for (id, half) in [
                    (CameraId::Left, &input[..plane]),
                    (CameraId::Right, &input[plane..]),
                ] {
                    inputs[id.index()] = Some(Preview {
                        width: size,
                        height: size,
                        pixels: plane_to_gray(half, size, 1.0),
                        generation,
                    });
                }
            }
            let mut raw = match model.infer(input) {
                Ok(raw) => raw,
                Err(e) => {
                    let message = pacing.failed(LABEL, &e);
                    lock(&SHARED.eyes).model.error = Some(message);
                    continue;
                }
            };
            if pacing.succeeded(LABEL) {
                lock(&SHARED.eyes).model.error = None;
            }
            etvr_eyes::mirror_missing(&mut raw, left.is_some(), right.is_some());
            let now = Instant::now();
            let mut values = raw;
            filters.filter(&mut values, now, &eyes.filter);
            let settings = observe_eyes(&values, eyes.settings);
            let state = etvr_eyes::compute(&values, &settings);
            *lock(&SHARED.eyes_estimate) = Some((state, settings.limits, now));
            let mut telemetry = lock(&SHARED.eyes);
            telemetry.raw = raw;
            telemetry.state = state;
            telemetry.present = [left.is_some(), right.is_some()];
            pacing.record(&mut telemetry.model, started);
        }
    }
}

fn observe_eyes(
    values: &[f32; etvr_eyes::OUTPUT_COUNT],
    settings: etvr_eyes::EyeSettings,
) -> etvr_eyes::EyeSettings {
    let recenter = SHARED.recenter.swap(false, Ordering::Relaxed);
    let calibrating = SHARED.eyes_calibrating.load(Ordering::Relaxed);
    if !recenter && !calibrating {
        return settings;
    }
    let gaze = etvr_eyes::gaze_raw(values);
    if calibrating {
        let mut extent = lock(&SHARED.gaze_extent);
        match extent.as_mut() {
            Some(extent) => {
                for (axis, value) in extent.iter_mut().zip(gaze) {
                    axis.observe(value);
                }
            }
            None => *extent = Some(gaze.map(Extent::new)),
        }
    }
    let updated = {
        let mut config = lock(&SHARED.config);
        let settings = &mut config.eyes.settings;
        if calibrating {
            let expressions = etvr_eyes::expressions(values);
            if SHARED.eyes_calibration_fresh.swap(false, Ordering::Relaxed) {
                settings.expressions.begin(&expressions);
            } else {
                settings.expressions.observe(&expressions);
            }
        }
        if recenter {
            settings.gaze.left_x.offset = gaze[0];
            settings.gaze.right_x.offset = gaze[1];
            settings.gaze.y.offset = gaze[2];
            *settings = std::mem::take(settings).sanitized();
        }
        settings.clone()
    };
    if recenter {
        log::info!(
            "ETVR camera: gaze recentred (left {:+.2}, right {:+.2}, vertical {:+.2})",
            gaze[0],
            gaze[1],
            gaze[2]
        );
        if let Err(e) = super::save_config() {
            log::warn!("ETVR camera: could not save the recentred gaze: {e:#}");
        }
    }
    updated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_keeps_only_the_latest_frame_and_wakes_waiters() {
        let hub = Hub::default();
        let mut seen = [0u64; 3];
        assert!(!hub.wait(&[CameraId::Mouth], &mut seen, Duration::from_millis(1)));
        hub.put(CameraId::Left, Image::new(1, 1, 1, vec![1]).unwrap());
        hub.put(CameraId::Left, Image::new(1, 1, 1, vec![2]).unwrap());
        assert!(!hub.wait(&[CameraId::Mouth], &mut seen, Duration::from_millis(1)));
        assert!(hub.wait(
            &[CameraId::Left, CameraId::Right],
            &mut seen,
            Duration::from_millis(1)
        ));
        assert_eq!(hub.latest(CameraId::Left).unwrap().data, vec![2]);
        assert!(!hub.wait(&[CameraId::Left], &mut seen, Duration::from_millis(1)));
        hub.clear(CameraId::Left);
        assert!(hub.latest(CameraId::Left).is_none());

        let hub = Arc::new(Hub::default());
        let producer = hub.clone();
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            producer.put(CameraId::Mouth, Image::new(1, 1, 1, vec![3]).unwrap());
        });
        let mut seen = [0u64; 3];
        assert!(hub.wait(&[CameraId::Mouth], &mut seen, Duration::from_secs(5)));
        handle.join().unwrap();
    }
}
