use std::{
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        mpsc::{sync_channel, Receiver, SyncSender, TrySendError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use colored::{Color, Colorize};
use glam::{vec3, Vec3};
use once_cell::sync::Lazy;
use rosc::{OscPacket, OscType};
use strum::EnumCount;

use crate::core::{AppState, INSTRUCTIONS_END, INSTRUCTIONS_START};

use super::{
    face2_fb::face2_fb_to_unified,
    unified::{UnifiedExpressions, UnifiedTrackingData},
    FaceReceiver,
};

const STALE: Duration = Duration::from_secs(1);
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const MAX_BUNDLE_DEPTH: usize = 8;

static STA_FACE_ON: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "SL-FACE".color(Color::Green)).into());
static STA_FACE_OFF: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "SL-FACE".color(Color::Red)).into());
static STA_EYE_ON: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "SL-EYE".color(Color::Green)).into());
static STA_EYE_OFF: Lazy<Arc<str>> = Lazy::new(|| format!("{}", "SL-EYE".color(Color::Red)).into());

pub(crate) const FB_NAMES: [&str; 70] = [
    "BrowLowererL",
    "BrowLowererR",
    "CheekPuffL",
    "CheekPuffR",
    "CheekRaiserL",
    "CheekRaiserR",
    "CheekSuckL",
    "CheekSuckR",
    "ChinRaiserB",
    "ChinRaiserT",
    "DimplerL",
    "DimplerR",
    "EyesClosedL",
    "EyesClosedR",
    "EyesLookDownL",
    "EyesLookDownR",
    "EyesLookLeftL",
    "EyesLookLeftR",
    "EyesLookRightL",
    "EyesLookRightR",
    "EyesLookUpL",
    "EyesLookUpR",
    "InnerBrowRaiserL",
    "InnerBrowRaiserR",
    "JawDrop",
    "JawSidewaysLeft",
    "JawSidewaysRight",
    "JawThrust",
    "LidTightenerL",
    "LidTightenerR",
    "LipCornerDepressorL",
    "LipCornerDepressorR",
    "LipCornerPullerL",
    "LipCornerPullerR",
    "LipFunnelerLB",
    "LipFunnelerLT",
    "LipFunnelerRB",
    "LipFunnelerRT",
    "LipPressorL",
    "LipPressorR",
    "LipPuckerL",
    "LipPuckerR",
    "LipStretcherL",
    "LipStretcherR",
    "LipSuckLB",
    "LipSuckLT",
    "LipSuckRB",
    "LipSuckRT",
    "LipTightenerL",
    "LipTightenerR",
    "LipsToward",
    "LowerLipDepressorL",
    "LowerLipDepressorR",
    "MouthLeft",
    "MouthRight",
    "NoseWrinklerL",
    "NoseWrinklerR",
    "OuterBrowRaiserL",
    "OuterBrowRaiserR",
    "UpperLidRaiserL",
    "UpperLidRaiserR",
    "UpperLipRaiserL",
    "UpperLipRaiserR",
    "TongueTipInterdental",
    "TongueTipAlveolar",
    "TongueFrontDorsalPalate",
    "TongueMidDorsalPalate",
    "TongueBackDorsalPalate",
    "TongueOut",
    "TongueRetreat",
];

const FB_EYES_CLOSED: [usize; 2] = [12, 13];

pub(crate) fn fb_index(name: &str) -> Option<usize> {
    FB_NAMES.iter().position(|candidate| *candidate == name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Region {
    Lower,
    Upper,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EyeParam {
    Lid,
    LidExpandedSqueeze,
    SqueezeToggle,
    WidenToggle,
    X,
    Y,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SteamLinkAddr {
    FaceWeight(usize),
    FaceConfidence(Region),
    GazePoint,
    Eye(Side, EyeParam),
    EyesClosedAmount,
    CenterVec,
}

pub(crate) fn parse_address(addr: &str) -> Option<SteamLinkAddr> {
    if let Some(name) = addr.strip_prefix("/sl/xrfb/facew/") {
        return fb_index(name).map(SteamLinkAddr::FaceWeight);
    }
    if let Some(region) = addr.strip_prefix("/sl/xrfb/facec/") {
        return match region {
            "LowerFace" => Some(SteamLinkAddr::FaceConfidence(Region::Lower)),
            "UpperFace" => Some(SteamLinkAddr::FaceConfidence(Region::Upper)),
            _ => None,
        };
    }
    match addr {
        "/sl/eyeTrackedGazePoint" => return Some(SteamLinkAddr::GazePoint),
        "/tracking/eye/EyesClosedAmount" => return Some(SteamLinkAddr::EyesClosedAmount),
        "/tracking/eye/CenterVecFull" => return Some(SteamLinkAddr::CenterVec),
        _ => {}
    }
    let param = addr.strip_prefix("/avatar/parameters/")?;
    let (side, rest) = match param.strip_prefix("Left") {
        Some(rest) => (Side::Left, rest),
        None => (Side::Right, param.strip_prefix("Right")?),
    };
    let kind = match rest {
        "EyeLid" => EyeParam::Lid,
        "EyeLidExpandedSqueeze" => EyeParam::LidExpandedSqueeze,
        "EyeSqueezeToggle" => EyeParam::SqueezeToggle,
        "EyeWidenToggle" => EyeParam::WidenToggle,
        "EyeX" => EyeParam::X,
        "EyeY" => EyeParam::Y,
        _ => return None,
    };
    Some(SteamLinkAddr::Eye(side, kind))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SteamLinkEvent {
    Weight(usize, f32),
    Confidence(Region, f32),
    Gaze(Vec3),
    EyeLid(Side, f32),
    EyesClosed(f32),
}

fn float_arg(arg: &OscType) -> Option<f32> {
    let value = match arg {
        OscType::Float(v) => *v,
        OscType::Double(v) => *v as f32,
        OscType::Int(v) => *v as f32,
        OscType::Bool(v) => f32::from(u8::from(*v)),
        _ => return None,
    };
    value.is_finite().then_some(value)
}

pub(crate) fn collect_events(packet: &OscPacket, out: &mut Vec<SteamLinkEvent>) {
    collect_inner(packet, out, 0);
}

fn collect_inner(packet: &OscPacket, out: &mut Vec<SteamLinkEvent>, depth: usize) {
    let message = match packet {
        OscPacket::Bundle(bundle) => {
            if depth < MAX_BUNDLE_DEPTH {
                for inner in &bundle.content {
                    collect_inner(inner, out, depth + 1);
                }
            }
            return;
        }
        OscPacket::Message(message) => message,
    };
    let Some(addr) = parse_address(&message.addr) else {
        return;
    };
    let first = message.args.first().and_then(float_arg);
    let event = match addr {
        SteamLinkAddr::FaceWeight(index) => first.map(|v| SteamLinkEvent::Weight(index, v)),
        SteamLinkAddr::FaceConfidence(region) => {
            first.map(|v| SteamLinkEvent::Confidence(region, v))
        }
        SteamLinkAddr::GazePoint => {
            let values: Vec<f32> = message.args.iter().filter_map(float_arg).collect();
            match values[..] {
                [x, y, z, ..] => Some(SteamLinkEvent::Gaze(vec3(x, y, z))),
                _ => None,
            }
        }
        SteamLinkAddr::Eye(side, EyeParam::Lid) => first.map(|v| SteamLinkEvent::EyeLid(side, v)),
        SteamLinkAddr::EyesClosedAmount => first.map(SteamLinkEvent::EyesClosed),
        _ => None,
    };
    if let Some(event) = event {
        out.push(event);
    }
}

pub(crate) fn gaze_to_euler(point: Vec3) -> Option<Vec3> {
    let length = point.length();
    if !length.is_finite() || length < 1e-4 {
        return None;
    }
    let d = point / length;
    let pitch = d.y.atan2((d.x * d.x + d.z * d.z).sqrt());
    let yaw = (-d.x).atan2(-d.z);
    (pitch.is_finite() && yaw.is_finite()).then(|| vec3(pitch, yaw, 0.0))
}

type Stamped = Option<(f32, Instant)>;

fn fresh(value: &Stamped, now: Instant) -> Option<f32> {
    value.and_then(|(v, at)| (now.duration_since(at) < STALE).then_some(v))
}

pub(super) struct SteamLinkReceiver {
    listen_port: u16,
    sender: SyncSender<Vec<SteamLinkEvent>>,
    receiver: Receiver<Vec<SteamLinkEvent>>,
    weights: [Stamped; 70],
    confidences: [Stamped; 2],
    eyelids: [Stamped; 2],
    eyes_closed: Stamped,
    gaze: Option<(Vec3, Instant)>,
    face_was_fresh: bool,
}

impl SteamLinkReceiver {
    pub fn new(listen_port: u16) -> Self {
        let (sender, receiver) = sync_channel(256);
        Self {
            listen_port,
            sender,
            receiver,
            weights: [None; 70],
            confidences: [None; 2],
            eyelids: [None; 2],
            eyes_closed: None,
            gaze: None,
            face_was_fresh: false,
        }
    }

    fn ingest(&mut self, events: &[SteamLinkEvent], now: Instant) {
        for event in events {
            match *event {
                SteamLinkEvent::Weight(index, value) => {
                    if let Some(slot) = self.weights.get_mut(index) {
                        *slot = Some((value, now));
                    }
                }
                SteamLinkEvent::Confidence(region, value) => {
                    self.confidences[region as usize] = Some((value, now));
                }
                SteamLinkEvent::Gaze(point) => {
                    if let Some(euler) = gaze_to_euler(point) {
                        self.gaze = Some((euler, now));
                    }
                }
                SteamLinkEvent::EyeLid(side, value) => {
                    self.eyelids[side as usize] = Some((value, now));
                }
                SteamLinkEvent::EyesClosed(value) => {
                    self.eyes_closed = Some((value, now));
                }
            }
        }
    }

    fn apply(&mut self, data: &mut UnifiedTrackingData, now: Instant) -> (bool, bool) {
        let mut weights = [0f32; 70];
        let mut face_fresh = false;
        for (dst, cached) in weights.iter_mut().zip(self.weights.iter()) {
            if let Some(value) = fresh(cached, now) {
                *dst = value.clamp(0.0, 1.0);
                face_fresh = true;
            }
        }

        let mut eye_fresh = false;
        for (i, index) in FB_EYES_CLOSED.iter().enumerate() {
            if fresh(&self.weights[*index], now).is_some() {
                continue;
            }
            let fallback = fresh(&self.eyelids[i], now)
                .map(|open| 1.0 - open)
                .or_else(|| fresh(&self.eyes_closed, now));
            if let Some(closed) = fallback {
                weights[*index] = closed.clamp(0.0, 1.0);
                eye_fresh = true;
            }
        }

        if face_fresh || eye_fresh || self.face_was_fresh {
            if let Some(shapes) = face2_fb_to_unified(&weights) {
                data.shapes[..UnifiedExpressions::COUNT]
                    .copy_from_slice(&shapes[..UnifiedExpressions::COUNT]);
            }
        }
        self.face_was_fresh = face_fresh || eye_fresh;

        match self.gaze {
            Some((euler, at)) if now.duration_since(at) < STALE => {
                data.eyes = [Some(euler), Some(euler)];
                eye_fresh = true;
            }
            Some(_) => {
                self.gaze = None;
                data.eyes = [None, None];
            }
            None => {}
        }

        (face_fresh, eye_fresh)
    }
}

impl FaceReceiver for SteamLinkReceiver {
    fn start_loop(&mut self) {
        log::info!("{}", *INSTRUCTIONS_START);
        log::info!("");
        log::info!("Selected SteamVR Steam Link to provide face and eye data.");
        log::info!("");
        log::info!("In the headset, open Steam Link settings:");
        log::info!("• enable OSC, eye tracking sharing and face tracking sharing");
        log::info!(
            "• under Advanced Settings, set the {} to {}",
            "OSC Output Port".color(Color::BrightYellow),
            format!("{}", self.listen_port).color(Color::Cyan),
        );
        log::info!("");
        log::info!("Status bar tickers:");
        log::info!("• {} → face data is being received", *STA_FACE_ON);
        log::info!("• {} → eye data is being received", *STA_EYE_ON);
        log::info!("");
        log::info!("{}", *INSTRUCTIONS_END);
        let sender = self.sender.clone();
        let port = self.listen_port;
        let spawned = thread::Builder::new()
            .name("steamlink".into())
            .spawn(move || steamlink_loop(port, sender));
        if let Err(e) = spawned {
            log::error!("Steam Link: could not start the listener thread: {e}");
        }
    }

    fn receive(&mut self, data: &mut UnifiedTrackingData, state: &mut AppState) {
        let now = Instant::now();
        let batches: Vec<_> = self.receiver.try_iter().collect();
        for events in batches {
            self.ingest(&events, now);
        }
        let (face, eye) = self.apply(data, now);
        state.status.add_item(if face {
            STA_FACE_ON.clone()
        } else {
            STA_FACE_OFF.clone()
        });
        state.status.add_item(if eye {
            STA_EYE_ON.clone()
        } else {
            STA_EYE_OFF.clone()
        });
    }
}

fn steamlink_loop(port: u16, sender: SyncSender<Vec<SteamLinkEvent>>) {
    let mut backoff = MIN_BACKOFF;
    let mut last_error: Option<String> = None;
    loop {
        let socket = match UdpSocket::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))) {
            Ok(socket) => socket,
            Err(e) => {
                let message = e.to_string();
                if last_error.as_deref() != Some(message.as_str()) {
                    log::warn!(
                        "Steam Link: cannot listen on 127.0.0.1:{port}: {message}. Retrying with backoff."
                    );
                    last_error = Some(message);
                }
                thread::sleep(backoff);
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        if last_error.take().is_some() {
            log::info!("Steam Link: listening on 127.0.0.1:{port}");
        }
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let size = match socket.recv_from(&mut buf) {
                Ok((size, _)) => size,
                Err(e) => {
                    log::debug!("Steam Link: receive error: {e}");
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
            };
            let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..size]) else {
                log::debug!("Steam Link: dropped malformed OSC packet ({size} bytes)");
                continue;
            };
            let mut events = Vec::new();
            collect_events(&packet, &mut events);
            if events.is_empty() {
                continue;
            }
            match sender.try_send(events) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rosc::{OscBundle, OscMessage};
    use std::collections::HashSet;

    fn msg(addr: &str, args: Vec<OscType>) -> OscPacket {
        OscPacket::Message(OscMessage {
            addr: addr.into(),
            args,
        })
    }

    #[test]
    fn fb_names_are_unique() {
        let unique: HashSet<_> = FB_NAMES.iter().collect();
        assert_eq!(unique.len(), FB_NAMES.len());
        assert_eq!(fb_index("JawDrop"), Some(24));
        assert_eq!(fb_index("EyesClosedL"), Some(FB_EYES_CLOSED[0]));
        assert_eq!(fb_index("EyesClosedR"), Some(FB_EYES_CLOSED[1]));
        assert_eq!(fb_index("TongueOut"), Some(68));
        assert_eq!(fb_index("UpperLipRaiserR"), Some(62));
    }

    #[test]
    fn addresses_parse() {
        assert_eq!(
            parse_address("/sl/xrfb/facew/JawDrop"),
            Some(SteamLinkAddr::FaceWeight(24))
        );
        assert_eq!(
            parse_address("/sl/xrfb/facew/BrowLowererL"),
            Some(SteamLinkAddr::FaceWeight(0))
        );
        assert_eq!(
            parse_address("/sl/xrfb/facec/LowerFace"),
            Some(SteamLinkAddr::FaceConfidence(Region::Lower))
        );
        assert_eq!(
            parse_address("/sl/xrfb/facec/UpperFace"),
            Some(SteamLinkAddr::FaceConfidence(Region::Upper))
        );
        assert_eq!(
            parse_address("/sl/eyeTrackedGazePoint"),
            Some(SteamLinkAddr::GazePoint)
        );
        assert_eq!(
            parse_address("/avatar/parameters/LeftEyeLid"),
            Some(SteamLinkAddr::Eye(Side::Left, EyeParam::Lid))
        );
        assert_eq!(
            parse_address("/avatar/parameters/RightEyeLidExpandedSqueeze"),
            Some(SteamLinkAddr::Eye(
                Side::Right,
                EyeParam::LidExpandedSqueeze
            ))
        );
        assert_eq!(
            parse_address("/avatar/parameters/RightEyeWidenToggle"),
            Some(SteamLinkAddr::Eye(Side::Right, EyeParam::WidenToggle))
        );
        assert_eq!(
            parse_address("/avatar/parameters/LeftEyeSqueezeToggle"),
            Some(SteamLinkAddr::Eye(Side::Left, EyeParam::SqueezeToggle))
        );
        assert_eq!(
            parse_address("/tracking/eye/EyesClosedAmount"),
            Some(SteamLinkAddr::EyesClosedAmount)
        );
        for bad in [
            "",
            "/",
            "/sl/xrfb/facew/",
            "/sl/xrfb/facew/jawdrop",
            "/sl/xrfb/facew/JawDrop/extra",
            "/sl/xrfb/facec/Middle",
            "/avatar/parameters/LeftEyeLidd",
            "/avatar/parameters/EyeLid",
            "/avatar/parameters/UpEyeLid",
            "/sl/eyeTrackedGazePoint/x",
        ] {
            assert_eq!(parse_address(bad), None, "{bad}");
        }
    }

    #[test]
    fn events_collect_from_nested_bundles_and_skip_bad_args() {
        let packet = OscPacket::Bundle(OscBundle {
            timetag: (0, 1).into(),
            content: vec![
                msg("/sl/xrfb/facew/JawDrop", vec![OscType::Float(0.5)]),
                msg("/sl/xrfb/facew/JawThrust", vec![OscType::Float(f32::NAN)]),
                msg(
                    "/sl/xrfb/facew/MouthLeft",
                    vec![OscType::String("x".into())],
                ),
                msg("/sl/xrfb/facew/MouthRight", vec![]),
                OscPacket::Bundle(OscBundle {
                    timetag: (0, 1).into(),
                    content: vec![
                        msg(
                            "/sl/eyeTrackedGazePoint",
                            vec![
                                OscType::Float(0.0),
                                OscType::Float(0.0),
                                OscType::Float(-1.0),
                            ],
                        ),
                        msg("/sl/eyeTrackedGazePoint", vec![OscType::Float(1.0)]),
                        msg("/avatar/parameters/RightEyeLid", vec![OscType::Float(0.25)]),
                    ],
                }),
            ],
        });
        let bytes = rosc::encoder::encode(&packet).unwrap();
        let (_, decoded) = rosc::decoder::decode_udp(&bytes).unwrap();
        let mut events = Vec::new();
        collect_events(&decoded, &mut events);
        assert_eq!(
            events,
            vec![
                SteamLinkEvent::Weight(24, 0.5),
                SteamLinkEvent::Gaze(vec3(0.0, 0.0, -1.0)),
                SteamLinkEvent::EyeLid(Side::Right, 0.25),
            ]
        );
    }

    #[test]
    fn gaze_point_converts_to_visage_euler() {
        let forward = gaze_to_euler(vec3(0.0, 0.0, -2.0)).unwrap();
        assert!(forward.x.abs() < 1e-6 && forward.y.abs() < 1e-6);
        let up = gaze_to_euler(vec3(0.0, 1.0, -1.0)).unwrap();
        assert!((up.x - std::f32::consts::FRAC_PI_4).abs() < 1e-5);
        let right = gaze_to_euler(vec3(1.0, 0.0, -1.0)).unwrap();
        assert!((right.y + std::f32::consts::FRAC_PI_4).abs() < 1e-5);
        assert!(gaze_to_euler(Vec3::ZERO).is_none());
        assert!(gaze_to_euler(vec3(f32::NAN, 0.0, -1.0)).is_none());
    }

    #[test]
    fn stale_data_falls_back_to_neutral() {
        let mut receiver = SteamLinkReceiver::new(9015);
        let mut data = UnifiedTrackingData::default();
        let t0 = Instant::now();
        receiver.ingest(
            &[
                SteamLinkEvent::Weight(24, 0.8),
                SteamLinkEvent::Weight(68, 0.6),
                SteamLinkEvent::Gaze(vec3(0.0, 0.0, -1.0)),
            ],
            t0,
        );
        assert_eq!(receiver.apply(&mut data, t0), (true, true));
        assert_eq!(data.getu(UnifiedExpressions::JawOpen), 0.8);
        assert_eq!(data.getu(UnifiedExpressions::TongueOut), 0.6);
        assert!(data.eyes[0].is_some());

        let later = t0 + STALE * 2;
        assert_eq!(receiver.apply(&mut data, later), (false, false));
        assert_eq!(data.getu(UnifiedExpressions::JawOpen), 0.0);
        assert!(data.eyes[0].is_none());

        data.setu(UnifiedExpressions::JawOpen, 0.3);
        assert_eq!(receiver.apply(&mut data, later), (false, false));
        assert_eq!(data.getu(UnifiedExpressions::JawOpen), 0.3);
    }

    #[test]
    fn eyelid_params_fill_in_when_fb_blink_is_missing() {
        let mut receiver = SteamLinkReceiver::new(9015);
        let mut data = UnifiedTrackingData::default();
        let now = Instant::now();
        receiver.ingest(
            &[
                SteamLinkEvent::EyeLid(Side::Left, 0.2),
                SteamLinkEvent::Weight(13, 0.9),
                SteamLinkEvent::EyeLid(Side::Right, 1.0),
            ],
            now,
        );
        receiver.apply(&mut data, now);
        assert!((data.getu(UnifiedExpressions::EyeClosedLeft) - 0.8).abs() < 1e-6);
        assert_eq!(data.getu(UnifiedExpressions::EyeClosedRight), 0.9);
    }
}
