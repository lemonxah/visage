use std::{
    ops::Add,
    sync::Arc,
    time::{Duration, Instant},
};

use colored::{Color, Colorize};
use glam::{vec3, Affine3A, EulerRot, Quat};
use mint::{Quaternion, Vector3};
use once_cell::sync::Lazy;
use openxr as xr;
use strum::EnumCount;

use crate::core::{AppState, INSTRUCTIONS_END, INSTRUCTIONS_START, TRACK_ON};

#[cfg(feature = "qpro-tongue")]
use super::qpro_tongue::{QproTongue, TongueArgs};
use super::{
    htc::{htc_to_unified, HtcFacialData},
    unified::{UnifiedExpressions, UnifiedShapeAccessors, UnifiedTrackingData},
    vergence::Vergence,
    FaceReceiver,
};

static STA_GAZE: Lazy<Arc<str>> = Lazy::new(|| format!("{}", "GAZE".color(Color::Green)).into());
static STA_GAZE_OFF: Lazy<Arc<str>> = Lazy::new(|| format!("{}", "GAZE".color(Color::Red)).into());
static STA_FACE: Lazy<Arc<str>> = Lazy::new(|| format!("{}", "FACE".color(Color::Green)).into());
static STA_FACE_OFF: Lazy<Arc<str>> = Lazy::new(|| format!("{}", "FACE".color(Color::Red)).into());
static STA_XR_IDLE: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "XR IDLE".color(Color::Yellow)).into());

const XR_IDLE_RECONNECT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default, clap::Args)]
pub struct OpenXrArgs {
    #[arg(
        long,
        help = "Split the combined gaze into per-eye gaze using the per-eye look blendshapes"
    )]
    pub independent_eyes: bool,

    #[arg(
        long,
        value_name = "PACKAGE",
        num_args = 0..=1,
        default_missing_value = "org.meumeu.wivrn",
        help = "Open the WiVRn client on the headset over adb at launch (default package org.meumeu.wivrn)"
    )]
    pub launch_wivrn: Option<String>,

    #[cfg(feature = "babble")]
    #[arg(
        long,
        value_name = "PORT",
        num_args = 0..=1,
        default_missing_value = "9400",
        value_parser = clap::value_parser!(u16).range(1..),
        help = "Use Babble mouth data alongside OpenXR gaze, blinks and brows (default port 9400)"
    )]
    pub babble: Option<u16>,

    #[cfg(feature = "qpro-tongue")]
    #[command(flatten)]
    pub tongue: TongueArgs,

    #[cfg(feature = "cameras")]
    #[command(flatten)]
    pub cameras: super::camera::CameraArgs,
}

struct Extras {
    vergence: Option<Vergence>,
    #[cfg(feature = "qpro-tongue")]
    tongue: Option<QproTongue>,
}

#[cfg(feature = "babble")]
pub(super) struct OpenXrBabbleReceiver {
    openxr: OpenXrReceiver,
    openxr_data: UnifiedTrackingData,
    babble: super::babble::BabbleEtvrReceiver,
}

#[cfg(feature = "babble")]
impl OpenXrBabbleReceiver {
    pub fn new(args: OpenXrArgs, port: u16) -> Self {
        Self {
            openxr: OpenXrReceiver::new(args),
            openxr_data: UnifiedTrackingData::default(),
            babble: super::babble::BabbleEtvrReceiver::new(port),
        }
    }
}

#[cfg(feature = "babble")]
impl FaceReceiver for OpenXrBabbleReceiver {
    fn start_loop(&mut self) {
        self.openxr.start_loop();
        self.babble.start_mouth_loop();
    }

    fn receive(&mut self, data: &mut UnifiedTrackingData, app: &mut AppState) {
        self.openxr.receive(&mut self.openxr_data, app);
        // Keep the native baseline separate: Babble and calc_combined must not
        // overwrite it, so expired mouth values can fall back on the next tick.
        data.eyes = self.openxr_data.eyes;
        data.shapes.copy_from_slice(&self.openxr_data.shapes);
        self.babble.receive_mouth(data, app);
    }
}

pub struct OpenXrReceiver {
    state: Option<XrState>,
    last_attempt: Instant,
    last_error: Option<String>,
    extras: Extras,
}

const XR_RETRY: Duration = Duration::from_secs(5);

impl OpenXrReceiver {
    pub fn new(args: OpenXrArgs) -> Self {
        if let Some(package) = args.launch_wivrn.clone() {
            launch_wivrn(package);
        }
        Self {
            state: None,
            last_attempt: Instant::now(),
            last_error: None,
            extras: Extras {
                vergence: args.independent_eyes.then(Vergence::new),
                #[cfg(feature = "qpro-tongue")]
                tongue: QproTongue::new(args.tongue),
            },
        }
    }

    fn try_init(&mut self) {
        self.last_attempt = Instant::now();
        match XrState::new() {
            Ok(state) => {
                if self.last_error.take().is_some() {
                    log::info!("XR: reconnected");
                }
                self.state = Some(state);
            }
            Err(e) => {
                let message = format!("{e:#}");
                if self.last_error.as_deref() != Some(message.as_str()) {
                    log::warn!("XR: {message}; retrying every {}s", XR_RETRY.as_secs());
                    self.last_error = Some(message);
                }
            }
        }
    }
}

fn launch_wivrn(package: String) {
    let spawned = std::thread::Builder::new()
        .name("wivrn-launch".into())
        .spawn(move || {
            if let Err(e) = try_launch_wivrn(&package) {
                log::warn!("WiVRn: {e:#}");
            }
        });
    if let Err(e) = spawned {
        log::warn!("WiVRn: could not start the launcher thread: {e}");
    }
}

fn try_launch_wivrn(package: &str) -> anyhow::Result<()> {
    use anyhow::{bail, Context};
    use std::process::{Command, Stdio};

    let adb = std::env::var("ADB").unwrap_or_else(|_| "adb".into());
    let run = |args: &[&str]| -> anyhow::Result<String> {
        let output = Command::new(&adb)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("could not run {adb}"))?;
        if !output.status.success() {
            bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&output.stdout).replace('\r', ""))
    };

    let _ = run(&["start-server"]);
    if run(&["get-state"]).is_err() {
        bail!(
            "no adb device; plug in the headset and allow USB debugging to auto-launch the client"
        );
    }
    let resolved = run(&[
        "shell",
        "cmd",
        "package",
        "resolve-activity",
        "--brief",
        "-c",
        "android.intent.category.LAUNCHER",
        package,
    ])?;
    let Some(activity) = resolved
        .lines()
        .map(str::trim)
        .rfind(|line| line.contains('/'))
    else {
        bail!("package {package} not found on the headset");
    };
    log::info!("WiVRn: launching client {activity}");
    run(&["shell", "am", "start", "-n", activity])?;
    Ok(())
}

impl FaceReceiver for OpenXrReceiver {
    fn start_loop(&mut self) {
        log::info!("{}", *INSTRUCTIONS_START);
        log::info!("");
        log::info!("Using OpenXR (WiVRn/Monado) to provide face data.");
        log::info!(
            "It's normal to see {} if the HMD is not yet connected.",
            "errors".color(Color::Red)
        );
        log::info!("");
        log::info!("Status bar tickers:");
        log::info!("• {} → face data is being received", *STA_FACE);
        log::info!("• {} → eye data is being received", *STA_GAZE);
        log::info!("• {} → head & wrist data is being received", *TRACK_ON);
        log::info!("");
        log::info!("{}", *INSTRUCTIONS_END);
        self.try_init();
    }

    fn receive(&mut self, data: &mut UnifiedTrackingData, app: &mut AppState) {
        let Some(state) = self.state.as_mut() else {
            if self.last_attempt.add(XR_RETRY) < Instant::now() {
                self.try_init();
            }
            app.status.add_item(STA_GAZE_OFF.clone());
            app.status.add_item(STA_FACE_OFF.clone());
            return;
        };

        if let Err(e) = state.receive(data, app, &mut self.extras) {
            log::warn!("XR: {e:#}; reconnecting");
            self.state = None;
            self.last_error = Some(format!("{e:#}"));
            self.last_attempt = Instant::now();
            *data = UnifiedTrackingData::default();
        }
    }
}

pub(super) struct XrState {
    face_tracker_fb: Option<MyFaceTrackerFB>,
    face_tracker_htc: Option<MyFaceTrackerHTC>,
    face_tracker_android: Option<super::android_face::AndroidFaceTracker>,
    instance: xr::Instance,
    system: xr::SystemId,
    session: xr::Session<xr::Headless>,
    frame_waiter: xr::FrameWaiter,
    frame_stream: xr::FrameStream<xr::Headless>,
    stage_space: xr::Space,
    view_space: xr::Space,
    eye_space: xr::Space,
    aim_spaces: [xr::Space; 2],
    actions: xr::ActionSet,
    eye_action: xr::Action<xr::Posef>,
    aim_actions: [xr::Action<xr::Posef>; 2],
    events: xr::EventDataBuffer,
    session_running: bool,
    idle_since: Instant,
    eyes_closed_frames: u32,
    verg_label: Option<(i32, Arc<str>)>,
}

impl XrState {
    fn new() -> anyhow::Result<Self> {
        let (instance, system) = xr_init()?;

        let actions = instance.create_action_set("visage", "Visage", 0)?;

        let eye_action = actions.create_action("eye_gaze", "Eye Gaze", &[])?;
        let aim_actions = [
            actions.create_action("left_aim", "Left Aim", &[])?,
            actions.create_action("right_aim", "Right Aim", &[])?,
        ];

        let (session, frame_waiter, frame_stream) =
            unsafe { instance.create_session(system, &xr::headless::SessionCreateInfo {})? };

        instance.suggest_interaction_profile_bindings(
            instance.string_to_path("/interaction_profiles/khr/simple_controller")?,
            &[
                xr::Binding::new(
                    &aim_actions[0],
                    instance.string_to_path("/user/hand/left/input/aim/pose")?,
                ),
                xr::Binding::new(
                    &aim_actions[1],
                    instance.string_to_path("/user/hand/right/input/aim/pose")?,
                ),
            ],
        )?;

        instance.suggest_interaction_profile_bindings(
            instance.string_to_path("/interaction_profiles/ext/eye_gaze_interaction")?,
            &[xr::Binding::new(
                &eye_action,
                instance.string_to_path("/user/eyes_ext/input/gaze_ext/pose")?,
            )],
        )?;

        session.attach_action_sets(&[&actions])?;

        let stage_space =
            session.create_reference_space(xr::ReferenceSpaceType::STAGE, xr::Posef::IDENTITY)?;

        let view_space =
            session.create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)?;

        let eye_space = eye_action.create_space(&session, xr::Path::NULL, xr::Posef::IDENTITY)?;

        let aim_spaces = [
            aim_actions[0].create_space(&session, xr::Path::NULL, xr::Posef::IDENTITY)?,
            aim_actions[1].create_space(&session, xr::Path::NULL, xr::Posef::IDENTITY)?,
        ];

        let mut me = Self {
            instance,
            system,
            session,
            frame_waiter,
            frame_stream,
            face_tracker_fb: None,
            face_tracker_htc: None,
            face_tracker_android: None,
            stage_space,
            view_space,
            eye_space,
            aim_spaces,
            actions,
            eye_action,
            aim_actions,
            events: xr::EventDataBuffer::new(),
            session_running: false,
            idle_since: Instant::now(),
            eyes_closed_frames: 0,
            verg_label: None,
        };

        me.face_tracker_fb = MyFaceTrackerFB::new(&me)
            .map_err(|e| log::info!("FB_face_tracking2: {}", e))
            .ok();
        me.face_tracker_htc = MyFaceTrackerHTC::new(&me)
            .map_err(|e| log::info!("HTC_facial_tracking: {}", e))
            .ok();
        if me.face_tracker_fb.is_none() {
            me.face_tracker_android = super::android_face::AndroidFaceTracker::new(
                &me.instance,
                me.system,
                me.session.as_raw(),
            )
            .map_err(|e| log::info!("ANDROID_face_tracking: {}", e))
            .ok();
        }

        Ok(me)
    }

    fn load_properties<T>(&self, next: *mut T) -> xr::Result<()> {
        unsafe {
            let mut p = xr::sys::SystemProperties {
                ty: xr::sys::SystemProperties::TYPE,
                next: next as _,
                ..std::mem::zeroed()
            };
            let res = (self.instance.fp().get_system_properties)(
                self.instance.as_raw(),
                self.system,
                &mut p,
            );
            if res != xr::sys::Result::SUCCESS {
                return Err(res);
            }
            Ok(())
        }
    }

    fn receive(
        &mut self,
        data: &mut UnifiedTrackingData,
        state: &mut AppState,
        extras: &mut Extras,
    ) -> anyhow::Result<()> {
        while let Some(event) = self.instance.poll_event(&mut self.events)? {
            use xr::Event::*;
            match event {
                SessionStateChanged(e) => match e.state() {
                    xr::SessionState::READY => {
                        self.session
                            .begin(xr::ViewConfigurationType::PRIMARY_STEREO)?;
                        self.session_running = true;
                        log::info!("XrSession started.")
                    }
                    xr::SessionState::STOPPING => {
                        self.session.end()?;
                        self.session_running = false;
                        self.idle_since = Instant::now();
                        log::warn!("XrSession stopped.")
                    }
                    xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => {
                        anyhow::bail!("XR session exiting");
                    }
                    other => log::info!("XrSession state: {:?}", other),
                },
                InstanceLossPending(_) => {
                    anyhow::bail!("XR instance loss pending");
                }
                EventsLost(e) => {
                    log::warn!("lost {} events", e.lost_event_count());
                }
                _ => {}
            }
        }

        if !self.session_running {
            state.status.add_item(STA_XR_IDLE.clone());
            state.status.add_item(STA_GAZE_OFF.clone());
            state.status.add_item(STA_FACE_OFF.clone());
            if self.idle_since.elapsed() > XR_IDLE_RECONNECT {
                anyhow::bail!(
                    "XR session not running for {}s, reconnecting",
                    XR_IDLE_RECONNECT.as_secs()
                );
            }
            return Ok(());
        }

        let next_frame = xr::Time::from_nanos(
            self.instance.now()?.as_nanos()
                + (state.status.last_frame_time.max(0.03334) * 1_000_000_000f32) as i64,
        );

        self.session.sync_actions(&[(&self.actions).into()])?;

        let hmd_loc = self.view_space.locate(&self.stage_space, next_frame)?;
        if hmd_loc
            .location_flags
            .contains(xr::SpaceLocationFlags::POSITION_VALID)
        {
            state.tracking.head = to_affine(&hmd_loc);
            state.tracking.last_received = Instant::now();
        } else {
            // HMD is sleeping, close the avatar's eyes (unless face tracker can provide valid data)
            data.shapes.setu(UnifiedExpressions::EyeClosedLeft, 1.0);
            data.shapes.setu(UnifiedExpressions::EyeClosedRight, 1.0);
        }

        let aim_loc = self.aim_spaces[0].locate(&self.stage_space, next_frame)?;
        state.tracking.left_hand = to_affine(&aim_loc);
        let aim_loc = self.aim_spaces[1].locate(&self.stage_space, next_frame)?;
        state.tracking.right_hand = to_affine(&aim_loc);

        let mut gaze_fresh = false;
        let mut face_active = false;
        #[cfg(feature = "qpro-tongue")]
        let mut native_tongue = None;

        let eye_loc = self.eye_space.locate(&self.view_space, next_frame)?;
        if eye_loc.location_flags.contains(
            xr::SpaceLocationFlags::ORIENTATION_VALID | xr::SpaceLocationFlags::ORIENTATION_TRACKED,
        ) {
            let now_q = to_quat(eye_loc.pose.orientation);
            let (y, x, z) = now_q.to_euler(EulerRot::YXZ);
            let (y, x, z) = if x.is_finite() && y.is_finite() && z.is_finite() {
                (y, x, z)
            } else {
                (0.0, 0.0, 0.0)
            };

            let mut eye_closed = ((x.to_degrees() + 5.0) / -55.0).max(0.0);

            if let Some(last) = data.eyes[0] {
                let last_q = Quat::from_euler(EulerRot::YXZ, last.y, last.x, last.z);

                if last_q.angle_between(now_q).to_degrees() > 10.0 {
                    self.eyes_closed_frames = 5;
                }
            }

            if self.eyes_closed_frames > 0 {
                self.eyes_closed_frames -= 1;
                eye_closed = 1.0;
            }

            data.shapes
                .setu(UnifiedExpressions::EyeClosedLeft, eye_closed);
            data.shapes
                .setu(UnifiedExpressions::EyeClosedRight, eye_closed);

            data.eyes[0] = Some(vec3(x, y, z));
            data.eyes[1] = data.eyes[0];
            gaze_fresh = true;
            state.status.add_item(STA_GAZE.clone());
        } else {
            state.status.add_item(STA_GAZE_OFF.clone());
        }

        if let Some(face_tracker) = self.face_tracker_fb.as_ref() {
            let mut weights = [0f32; 70];
            let mut confidences = [0f32; 2];

            let is_valid = face_tracker.get_face_expression_weights(
                next_frame,
                &mut weights,
                &mut confidences,
            )?;

            if is_valid {
                if let Some(shapes) = super::face2_fb::face2_fb_to_unified(&weights) {
                    data.shapes[..=UnifiedExpressions::COUNT]
                        .copy_from_slice(&shapes[..=UnifiedExpressions::COUNT]);
                    #[cfg(feature = "qpro-tongue")]
                    {
                        native_tongue = Some(data.getu(UnifiedExpressions::TongueOut));
                    }
                }
                if gaze_fresh {
                    if let (Some(vergence), Some(combined), Some((left_x, right_x))) = (
                        extras.vergence.as_mut(),
                        data.eyes[0],
                        super::face2_fb::eye_look_x(&weights),
                    ) {
                        let [left, right] = vergence.split(combined, left_x, right_x);
                        data.eyes = [Some(left), Some(right)];
                        let tenths = ((right.y - left.y).to_degrees() * 10.0).round() as i32;
                        let label = match &self.verg_label {
                            Some((cached, label)) if *cached == tenths => label.clone(),
                            _ => {
                                let label: Arc<str> =
                                    format!("VERG:{:+.1}°", tenths as f32 / 10.0).into();
                                self.verg_label = Some((tenths, label.clone()));
                                label
                            }
                        };
                        state.status.add_item(label);
                    }
                }
                face_active = true;
            }
        };

        if let Some(face_tracker) = self.face_tracker_android.as_ref() {
            let shapes = face_tracker
                .get_parameters(next_frame)
                .and_then(|parameters| super::android_face::android_to_unified(&parameters));
            if let Some(shapes) = shapes {
                data.shapes[..UnifiedExpressions::COUNT]
                    .copy_from_slice(&shapes[..UnifiedExpressions::COUNT]);
                #[cfg(feature = "qpro-tongue")]
                {
                    native_tongue = Some(data.getu(UnifiedExpressions::TongueOut));
                }
                face_active = true;
            }
        }

        if let Some(face_tracker) = self.face_tracker_htc.as_ref() {
            let htc_data = face_tracker.get_expressions(next_frame);

            if htc_data.eye.is_some() || htc_data.lip.is_some() {
                let shapes = htc_to_unified(&htc_data);
                data.shapes[..=UnifiedExpressions::COUNT]
                    .copy_from_slice(&shapes[..=UnifiedExpressions::COUNT]);
                face_active = true;
            }
        }

        if face_active {
            state.status.add_item(STA_FACE.clone());
        } else if self.face_tracker_fb.is_some()
            || self.face_tracker_android.is_some()
            || self.face_tracker_htc.is_some()
        {
            state.status.add_item(STA_FACE_OFF.clone());
        }

        #[cfg(feature = "qpro-tongue")]
        if let Some(tongue) = extras.tongue.as_mut() {
            tongue.apply(native_tongue, data, &mut state.status);
        }

        Ok(())
    }
}

fn xr_init() -> anyhow::Result<(xr::Instance, xr::SystemId)> {
    let entry = xr::Entry::linked(&())?;

    let Ok(available_extensions) = entry.enumerate_extensions() else {
        anyhow::bail!("Failed to enumerate OpenXR extensions.");
    };

    anyhow::ensure!(
        available_extensions.mnd_headless,
        "Missing MND_headless extension."
    );

    let mut enabled_extensions = xr::ExtensionSet::default();
    enabled_extensions.mnd_headless = true;
    enabled_extensions.khr_convert_timespec_time = true;

    if available_extensions.ext_eye_gaze_interaction {
        enabled_extensions.ext_eye_gaze_interaction = true;
    } else {
        log::warn!("Missing EXT_eye_gaze_interaction extension. Is Monado/WiVRn up to date?");
    }

    if available_extensions.fb_face_tracking2 {
        enabled_extensions.fb_face_tracking2 = true;
    }

    if available_extensions.htc_facial_tracking {
        enabled_extensions.htc_facial_tracking = true;
    }

    if super::android_face::is_available(&available_extensions) {
        enabled_extensions.android_face_tracking = true;
    }

    let Ok(instance) = entry.create_instance(
        &xr::ApplicationInfo {
            api_version: xr::Version::new(1, 0, 0),
            application_name: "visage",
            application_version: 0,
            engine_name: "visage",
            engine_version: 0,
        },
        &enabled_extensions,
        &[],
        &(),
    ) else {
        anyhow::bail!("Failed to create OpenXR instance.");
    };

    let Ok(instance_props) = instance.properties() else {
        anyhow::bail!("Failed to query OpenXR instance properties.");
    };
    log::info!(
        "Using OpenXR runtime: {} {}",
        instance_props.runtime_name,
        instance_props.runtime_version
    );

    let Ok(system) = instance.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY) else {
        anyhow::bail!("Failed to access OpenXR HMD system.");
    };

    Ok((instance, system))
}

struct MyFaceTrackerFB {
    api: xr::raw::FaceTracking2FB,
    tracker: xr::sys::FaceTracker2FB,
}

impl MyFaceTrackerFB {
    pub fn new(xr_state: &XrState) -> anyhow::Result<Self> {
        if xr_state.instance.exts().fb_face_tracking2.is_none() {
            anyhow::bail!("Extension not supported.");
        }

        let mut props = xr::sys::SystemFaceTrackingProperties2FB {
            ty: xr::StructureType::SYSTEM_FACE_TRACKING_PROPERTIES2_FB,
            next: std::ptr::null_mut(),
            supports_visual_face_tracking: xr::sys::Bool32::from_raw(0),
            supports_audio_face_tracking: xr::sys::Bool32::from_raw(0),
        };

        xr_state.load_properties(&mut props)?;

        if props.supports_visual_face_tracking.into_raw() == 0 {
            anyhow::bail!("Unable to provide visual data.");
        }

        let api = unsafe {
            xr::raw::FaceTracking2FB::load(
                xr_state.session.instance().entry(),
                xr_state.session.instance().as_raw(),
            )?
        };

        let mut data_source = xr::sys::FaceTrackingDataSource2FB::VISUAL;

        let info = xr::sys::FaceTrackerCreateInfo2FB {
            ty: xr::StructureType::FACE_TRACKER_CREATE_INFO2_FB,
            next: std::ptr::null(),
            face_expression_set: xr::FaceExpressionSet2FB::DEFAULT,
            requested_data_source_count: 1,
            requested_data_sources: &mut data_source,
        };

        let mut tracker = xr::sys::FaceTracker2FB::default();

        let res =
            unsafe { (api.create_face_tracker2)(xr_state.session.as_raw(), &info, &mut tracker) };
        if res.into_raw() != 0 {
            anyhow::bail!("Could not initialize: {:?}", res);
        }

        log::info!("Using FB_face_tracking2 for face.");

        Ok(Self { api, tracker })
    }

    pub fn get_face_expression_weights(
        &self,
        time: xr::Time,
        weights: &mut [f32],
        confidences: &mut [f32],
    ) -> anyhow::Result<bool> {
        let mut expressions = xr::sys::FaceExpressionWeights2FB {
            ty: xr::StructureType::FACE_EXPRESSION_WEIGHTS2_FB,
            next: std::ptr::null_mut(),
            weight_count: weights.len() as _,
            weights: weights.as_mut_ptr(),
            confidence_count: confidences.len() as _,
            confidences: confidences.as_mut_ptr(),
            is_eye_following_blendshapes_valid: xr::sys::Bool32::from_raw(0),
            is_valid: xr::sys::Bool32::from_raw(0),
            data_source: xr::sys::FaceTrackingDataSource2FB::VISUAL,
            time,
        };

        let info = xr::sys::FaceExpressionInfo2FB {
            ty: xr::StructureType::FACE_EXPRESSION_INFO2_FB,
            next: std::ptr::null(),
            time,
        };

        let res = unsafe {
            (self.api.get_face_expression_weights2)(self.tracker, &info, &mut expressions)
        };
        if res.into_raw() != 0 {
            anyhow::bail!("Failed to get expression weights");
        }

        Ok(expressions.is_valid.into_raw() != 0)
    }
}

impl Drop for MyFaceTrackerFB {
    fn drop(&mut self) {
        unsafe {
            (self.api.destroy_face_tracker2)(self.tracker);
        }
    }
}

pub(super) struct MyFaceTrackerHTC {
    api: xr::raw::FacialTrackingHTC,
    eye_tracker: Option<xr::sys::FacialTrackerHTC>,
    lip_tracker: Option<xr::sys::FacialTrackerHTC>,
}

impl MyFaceTrackerHTC {
    pub fn new(xr_state: &XrState) -> anyhow::Result<Self> {
        if xr_state.instance.exts().htc_facial_tracking.is_none() {
            anyhow::bail!("Extension not supported.");
        }
        let mut props = xr::sys::SystemFacialTrackingPropertiesHTC {
            ty: xr::StructureType::SYSTEM_FACIAL_TRACKING_PROPERTIES_HTC,
            next: std::ptr::null_mut(),
            support_eye_facial_tracking: xr::sys::Bool32::from_raw(0),
            support_lip_facial_tracking: xr::sys::Bool32::from_raw(0),
        };

        xr_state.load_properties(&mut props)?;

        if props.support_eye_facial_tracking.into_raw()
            + props.support_lip_facial_tracking.into_raw()
            == 0
        {
            anyhow::bail!("Unable to provide lip/eye data.");
        }

        let api = unsafe {
            xr::raw::FacialTrackingHTC::load(
                xr_state.session.instance().entry(),
                xr_state.session.instance().as_raw(),
            )?
        };

        let mut info = xr::sys::FacialTrackerCreateInfoHTC {
            ty: xr::StructureType::FACIAL_TRACKER_CREATE_INFO_HTC,
            next: std::ptr::null(),
            facial_tracking_type: xr::sys::FacialTrackingTypeHTC::EYE_DEFAULT,
        };

        let eye_tracker = if props.support_eye_facial_tracking.into_raw() != 0 {
            let mut eye_tracker = xr::sys::FacialTrackerHTC::default();

            let res = unsafe {
                (api.create_facial_tracker)(xr_state.session.as_raw(), &info, &mut eye_tracker)
            };
            if res.into_raw() != 0 {
                anyhow::bail!("Could not initialize upper face tracker: {:?}", res);
            }
            Some(eye_tracker)
        } else {
            None
        };

        let lip_tracker = if props.support_lip_facial_tracking.into_raw() != 0 {
            info.facial_tracking_type = xr::sys::FacialTrackingTypeHTC::LIP_DEFAULT;

            let mut lip_tracker = xr::sys::FacialTrackerHTC::default();

            let res = unsafe {
                (api.create_facial_tracker)(xr_state.session.as_raw(), &info, &mut lip_tracker)
            };
            if res.into_raw() != 0 {
                anyhow::bail!("Could not initialize lower face tracker: {:?}", res);
            }
            Some(lip_tracker)
        } else {
            None
        };

        log::info!("Using HTC_facial_tracking for face.");

        Ok(Self {
            api,
            eye_tracker,
            lip_tracker,
        })
    }

    fn get_expressions_internal<const E: usize>(
        &self,
        tracker: xr::sys::FacialTrackerHTC,
        sample_time: xr::Time,
    ) -> Option<[f32; E]> {
        let mut arr = [0f32; E];
        let mut info = xr::sys::FacialExpressionsHTC {
            ty: xr::StructureType::FACIAL_EXPRESSIONS_HTC,
            next: std::ptr::null_mut(),
            sample_time,
            is_active: xr::sys::Bool32::from_raw(0),
            expression_count: arr.len() as _,
            expression_weightings: arr.as_mut_ptr(),
        };

        let res = unsafe { (self.api.get_facial_expressions)(tracker, &mut info) };
        if res.into_raw() != 0 {
            log::error!("Failed to get HTC facial expression weights");
            return None;
        }

        if info.is_active.into_raw() != 0 {
            Some(arr)
        } else {
            None
        }
    }

    pub fn get_expressions(&self, sample_time: xr::Time) -> HtcFacialData {
        HtcFacialData {
            eye: self
                .eye_tracker
                .and_then(|t| self.get_expressions_internal(t, sample_time)),
            lip: self
                .lip_tracker
                .and_then(|t| self.get_expressions_internal(t, sample_time)),
        }
    }
}

impl Drop for MyFaceTrackerHTC {
    fn drop(&mut self) {
        unsafe {
            if let Some(tracker) = self.eye_tracker.take() {
                (self.api.destroy_facial_tracker)(tracker);
            }
            if let Some(tracker) = self.lip_tracker.take() {
                (self.api.destroy_facial_tracker)(tracker);
            }
        }
    }
}

fn to_quat(p: xr::Quaternionf) -> Quat {
    let q: Quaternion<f32> = p.into();
    q.into()
}

fn to_affine(loc: &xr::SpaceLocation) -> Affine3A {
    let t: Vector3<f32> = loc.pose.position.into();
    Affine3A::from_rotation_translation(to_quat(loc.pose.orientation), t.into())
}
