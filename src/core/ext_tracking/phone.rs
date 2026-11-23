use std::{
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        mpsc::{sync_channel, Receiver, SyncSender, TrySendError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use colored::{Color, Colorize};
use once_cell::sync::Lazy;
use serde::Deserialize;
use strum::EnumCount;

use crate::core::{AppState, INSTRUCTIONS_END, INSTRUCTIONS_START};

use super::{
    arkit::{arkit_to_unified, ArkitShape, ArkitWeights, ARKIT_COUNT},
    unified::{UnifiedExpressions, UnifiedTrackingData, NUM_SHAPES},
    FaceReceiver,
};

const STALE: Duration = Duration::from_secs(1);
const READ_TIMEOUT: Duration = Duration::from_millis(250);
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const MAX_PACKET: usize = 16 * 1024;
const MAX_ENTRIES: usize = 128;

const IFM_PORT: u16 = 49983;
const IFM_HANDSHAKE: &str =
    "iFacialMocap_sahuasouryya9218sauhuiayeta91555dy3719|sendDataVersion=v2";
const IFM_HANDSHAKE_EVERY: Duration = Duration::from_secs(2);

const VTS_PORT: u16 = 21412;
const VTS_REQUEST_EVERY: Duration = Duration::from_secs(1);
const VTS_REQUEST_SECONDS: f32 = 5.0;

static STA_ON: Lazy<Arc<str>> = Lazy::new(|| format!("{}", "PHONE".color(Color::Green)).into());
static STA_OFF: Lazy<Arc<str>> = Lazy::new(|| format!("{}", "PHONE".color(Color::Red)).into());

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum PhoneProtocol {
    #[default]
    #[value(name = "ifacialmocap", alias = "ifm")]
    IFacialMocap,
    #[value(name = "vts", alias = "meowface")]
    Vts,
}

impl PhoneProtocol {
    fn default_port(self) -> u16 {
        match self {
            PhoneProtocol::IFacialMocap => IFM_PORT,
            PhoneProtocol::Vts => VTS_PORT,
        }
    }

    fn label(self) -> &'static str {
        match self {
            PhoneProtocol::IFacialMocap => "iFacialMocap",
            PhoneProtocol::Vts => "VTube Studio / MeowFace",
        }
    }
}

#[derive(Clone, Debug, clap::Args)]
pub struct PhoneArgs {
    #[arg(
        value_name = "PHONE_IP",
        help = "IP address of the phone running the tracking app"
    )]
    pub ip: IpAddr,

    #[arg(
        long,
        value_enum,
        default_value_t = PhoneProtocol::IFacialMocap,
        help = "Phone app protocol: ifacialmocap, or vts (VTube Studio iOS and MeowFace)"
    )]
    pub protocol: PhoneProtocol,

    #[arg(
        long,
        value_name = "PORT",
        value_parser = clap::value_parser!(u16).range(1..),
        help = "UDP port the phone app listens on (default 49983 for iFacialMocap, 21412 for VTS)"
    )]
    pub phone_port: Option<u16>,

    #[arg(
        long,
        value_name = "PORT",
        value_parser = clap::value_parser!(u16).range(1..),
        help = "Local UDP port to receive on (default 49983 for iFacialMocap, 21412 for VTS)"
    )]
    pub listen: Option<u16>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PhoneFrame {
    pub weights: ArkitWeights,
    pub shapes_seen: usize,
}

fn percent(value: f32) -> f32 {
    if value.is_finite() {
        (value / 100.0).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

pub(crate) fn parse_ifacialmocap(text: &str) -> Option<PhoneFrame> {
    let mut weights = [0f32; ARKIT_COUNT];
    let mut shapes_seen = 0;
    for token in text.split('|').take(MAX_ENTRIES) {
        let token = token.trim();
        if token.is_empty() || token.starts_with('=') || token.contains('#') {
            continue;
        }
        let Some((name, value)) = token.split_once('&').or_else(|| token.split_once('-')) else {
            continue;
        };
        let Some(shape) = ArkitShape::from_name(name) else {
            continue;
        };
        let Ok(value) = value.trim().parse::<f32>() else {
            continue;
        };
        weights[shape as usize] = percent(value);
        shapes_seen += 1;
    }
    (shapes_seen > 0).then_some(PhoneFrame {
        weights,
        shapes_seen,
    })
}

#[derive(Deserialize)]
struct VtsEntry {
    #[serde(default)]
    k: String,
    #[serde(default)]
    v: f32,
}

#[derive(Deserialize)]
struct VtsPacket {
    #[serde(rename = "FaceFound", default)]
    face_found: bool,
    #[serde(rename = "BlendShapes", default)]
    blend_shapes: Vec<VtsEntry>,
}

pub(crate) fn parse_vts(text: &str) -> Option<PhoneFrame> {
    let packet: VtsPacket = serde_json::from_str(text).ok()?;
    if !packet.face_found {
        return None;
    }
    let mut weights = [0f32; ARKIT_COUNT];
    let mut shapes_seen = 0;
    for entry in packet.blend_shapes.iter().take(MAX_ENTRIES) {
        let Some(shape) = ArkitShape::from_name(&entry.k) else {
            continue;
        };
        weights[shape as usize] = super::arkit::sanitize(entry.v);
        shapes_seen += 1;
    }
    (shapes_seen > 0).then_some(PhoneFrame {
        weights,
        shapes_seen,
    })
}

fn vts_request(listen_port: u16) -> String {
    serde_json::json!({
        "messageType": "iOSTrackingDataRequest",
        "time": VTS_REQUEST_SECONDS,
        "sentBy": "Visage",
        "ports": [listen_port],
    })
    .to_string()
}

pub(super) struct PhoneReceiver {
    args: PhoneArgs,
    sender: SyncSender<PhoneFrame>,
    receiver: Receiver<PhoneFrame>,
    last: Option<(PhoneFrame, Instant)>,
    was_fresh: bool,
}

impl PhoneReceiver {
    pub fn new(args: PhoneArgs) -> Self {
        let (sender, receiver) = sync_channel(16);
        Self {
            args,
            sender,
            receiver,
            last: None,
            was_fresh: false,
        }
    }

    fn apply(&mut self, data: &mut UnifiedTrackingData, now: Instant) -> bool {
        let fresh = match &self.last {
            Some((frame, at)) if now.duration_since(*at) < STALE => {
                let shapes = arkit_to_unified(&frame.weights);
                data.shapes[..UnifiedExpressions::COUNT]
                    .copy_from_slice(&shapes[..UnifiedExpressions::COUNT]);
                true
            }
            _ => false,
        };
        if !fresh && self.was_fresh {
            data.shapes[..UnifiedExpressions::COUNT]
                .copy_from_slice(&[0.0; NUM_SHAPES][..UnifiedExpressions::COUNT]);
            self.last = None;
        }
        self.was_fresh = fresh;
        fresh
    }
}

impl FaceReceiver for PhoneReceiver {
    fn start_loop(&mut self) {
        let protocol = self.args.protocol;
        let listen = self.args.listen.unwrap_or(protocol.default_port());
        let phone = SocketAddr::new(
            self.args.ip,
            self.args.phone_port.unwrap_or(protocol.default_port()),
        );
        log::info!("{}", *INSTRUCTIONS_START);
        log::info!("");
        log::info!(
            "Selected {} on {} to provide ARKit face data.",
            protocol.label().color(Color::BrightYellow),
            phone.to_string().color(Color::Cyan)
        );
        match protocol {
            PhoneProtocol::IFacialMocap => {
                log::info!("• Open iFacialMocap on the phone; Visage connects to it automatically");
            }
            PhoneProtocol::Vts => {
                log::info!(
                    "• VTube Studio iOS: enable {} in settings",
                    "3rd Party PC Clients".color(Color::BrightYellow)
                );
                log::info!("• MeowFace: keep the app open on the same network; Visage requests data from it");
            }
        }
        log::info!(
            "• Allow UDP port {} through the firewall on this PC",
            format!("{listen}").color(Color::Cyan)
        );
        log::info!("");
        log::info!("Status bar tickers:");
        log::info!("• {} → face data is being received", *STA_ON);
        log::info!("");
        log::info!("{}", *INSTRUCTIONS_END);

        let sender = self.sender.clone();
        let spawned = thread::Builder::new()
            .name("phone".into())
            .spawn(move || phone_loop(protocol, phone, listen, sender));
        if let Err(e) = spawned {
            log::error!("Phone: could not start the receiver thread: {e}");
        }
    }

    fn receive(&mut self, data: &mut UnifiedTrackingData, state: &mut AppState) {
        if let Some(frame) = self.receiver.try_iter().last() {
            self.last = Some((frame, Instant::now()));
        }
        let fresh = self.apply(data, Instant::now());
        state.status.add_item(if fresh {
            STA_ON.clone()
        } else {
            STA_OFF.clone()
        });
    }
}

fn phone_loop(
    protocol: PhoneProtocol,
    phone: SocketAddr,
    listen: u16,
    sender: SyncSender<PhoneFrame>,
) {
    let mut backoff = MIN_BACKOFF;
    let mut last_error: Option<String> = None;
    loop {
        let bind = SocketAddr::from((Ipv4Addr::UNSPECIFIED, listen));
        let socket = match UdpSocket::bind(bind).and_then(|socket| {
            socket.set_read_timeout(Some(READ_TIMEOUT))?;
            Ok(socket)
        }) {
            Ok(socket) => socket,
            Err(e) => {
                let message = e.to_string();
                if last_error.as_deref() != Some(message.as_str()) {
                    log::warn!(
                        "Phone: cannot listen on UDP {listen}: {message}. Retrying with backoff."
                    );
                    last_error = Some(message);
                }
                thread::sleep(backoff);
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        if last_error.take().is_some() {
            log::info!("Phone: listening on UDP {listen}");
        }
        backoff = MIN_BACKOFF;
        if !receive_frames(&socket, protocol, phone, listen, &sender) {
            return;
        }
        thread::sleep(backoff);
    }
}

fn receive_frames(
    socket: &UdpSocket,
    protocol: PhoneProtocol,
    phone: SocketAddr,
    listen: u16,
    sender: &SyncSender<PhoneFrame>,
) -> bool {
    let request = match protocol {
        PhoneProtocol::IFacialMocap => IFM_HANDSHAKE.to_string(),
        PhoneProtocol::Vts => vts_request(listen),
    };
    let request_every = match protocol {
        PhoneProtocol::IFacialMocap => IFM_HANDSHAKE_EVERY,
        PhoneProtocol::Vts => VTS_REQUEST_EVERY,
    };
    let mut buf = vec![0u8; MAX_PACKET];
    let mut last_request: Option<Instant> = None;
    let mut last_frame: Option<Instant> = None;
    let mut announced = false;
    let mut send_warned = false;
    loop {
        let streaming = last_frame.is_some_and(|at| at.elapsed() < STALE * 2);
        let due = last_request.is_none_or(|at| at.elapsed() >= request_every);
        let wants_request = match protocol {
            PhoneProtocol::IFacialMocap => !streaming,
            PhoneProtocol::Vts => true,
        };
        if due && wants_request {
            last_request = Some(Instant::now());
            match socket.send_to(request.as_bytes(), phone) {
                Ok(_) => send_warned = false,
                Err(e) if !send_warned => {
                    log::warn!("Phone: cannot reach {phone}: {e}");
                    send_warned = true;
                }
                Err(e) => log::debug!("Phone: cannot reach {phone}: {e}"),
            }
        }

        let (size, from) = match socket.recv_from(&mut buf) {
            Ok(received) => received,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
                log::debug!("Phone: {phone} refused the request");
                continue;
            }
            Err(e) => {
                log::warn!("Phone: receive failed: {e}; reopening the socket");
                return true;
            }
        };
        if from.ip() != phone.ip() || size >= MAX_PACKET {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&buf[..size]) else {
            continue;
        };
        let frame = match protocol {
            PhoneProtocol::IFacialMocap => parse_ifacialmocap(text),
            PhoneProtocol::Vts => parse_vts(text),
        };
        let Some(frame) = frame else {
            log::debug!("Phone: ignored a {size} byte packet");
            continue;
        };
        last_frame = Some(Instant::now());
        if !announced {
            log::info!(
                "Phone: receiving {} blendshapes from {}",
                frame.shapes_seen,
                phone
            );
            announced = true;
        }
        match sender.try_send(frame) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ext_tracking::unified::UnifiedShapeAccessors;

    const IFM_V1: &str = "mouthSmile_R-35|jawOpen-12|eyeBlink_L-7|tongueOut-0|=head#-2.5,-6.0,1.2,0.1,-0.3,0.6|rightEye#1.5,2.0,0.2|leftEye#1.4,-1.6,-0.1|";
    const IFM_V2: &str = "mouthSmile_R&35|jawOpen&12|eyeBlink_L&7|=head#-2.5,-6.0,1.2,0.1,-0.3,0.6|rightEye#1.5,2.0,0.2|leftEye#1.4,-1.6,-0.1|";

    fn weight(frame: &PhoneFrame, shape: ArkitShape) -> f32 {
        frame.weights[shape as usize]
    }

    #[test]
    fn ifacialmocap_v1_and_v2_frames_parse() {
        for line in [IFM_V1, IFM_V2] {
            let frame = parse_ifacialmocap(line).unwrap();
            assert!((weight(&frame, ArkitShape::MouthSmileRight) - 0.35).abs() < 1e-6);
            assert!((weight(&frame, ArkitShape::JawOpen) - 0.12).abs() < 1e-6);
            assert!((weight(&frame, ArkitShape::EyeBlinkLeft) - 0.07).abs() < 1e-6);
            assert_eq!(weight(&frame, ArkitShape::BrowInnerUp), 0.0);
        }
        assert_eq!(parse_ifacialmocap(IFM_V1).unwrap().shapes_seen, 4);
    }

    #[test]
    fn ifacialmocap_clamps_out_of_range_values() {
        let frame =
            parse_ifacialmocap("jawOpen-250|mouthClose--40|eyeWide_R&-5|cheekPuff-50.5").unwrap();
        assert_eq!(weight(&frame, ArkitShape::JawOpen), 1.0);
        assert_eq!(weight(&frame, ArkitShape::MouthClose), 0.0);
        assert_eq!(weight(&frame, ArkitShape::EyeWideRight), 0.0);
        assert!((weight(&frame, ArkitShape::CheekPuff) - 0.505).abs() < 1e-6);
        let frame = parse_ifacialmocap("jawOpen-NaN|mouthClose-inf|eyeWide_R-3").unwrap();
        assert_eq!(weight(&frame, ArkitShape::JawOpen), 0.0);
        assert_eq!(weight(&frame, ArkitShape::MouthClose), 0.0);
        assert!((weight(&frame, ArkitShape::EyeWideRight) - 0.03).abs() < 1e-6);
    }

    #[test]
    fn ifacialmocap_rejects_malformed_and_keeps_partial() {
        for bad in [
            "",
            "|||",
            "garbage",
            "=head#1,2,3,4,5,6|rightEye#1,2,3|leftEye#1,2,3|",
            "jawOpen|mouthClose-|-5|notAShape-5",
            "jawOpen-abc",
            "\u{0}\u{1}\u{2}",
        ] {
            assert_eq!(parse_ifacialmocap(bad), None, "{bad:?}");
        }
        let partial = parse_ifacialmocap("jawOpen-50|mouthSmi").unwrap();
        assert_eq!(partial.shapes_seen, 1);
        assert_eq!(weight(&partial, ArkitShape::JawOpen), 0.5);
        let long = "jawOpen-10|".repeat(10_000);
        assert_eq!(parse_ifacialmocap(&long).unwrap().shapes_seen, MAX_ENTRIES);
    }

    #[test]
    fn vts_json_parses_and_tolerates_unknown_fields() {
        let json = r#"{"Timestamp":1700000000000,"Hotkey":-1,"FaceFound":true,
            "Rotation":{"x":1.0,"y":2.0,"z":3.0},"Position":{"x":0,"y":0,"z":0},
            "EyeLeft":{"x":0,"y":0,"z":0},"EyeRight":{"x":0,"y":0,"z":0},
            "NewField":[1,2,3],
            "BlendShapes":[{"k":"EyeBlinkLeft","v":0.5},{"k":"JawOpen","v":1.7},
            {"k":"MouthSmileRight","v":-0.2},{"k":"Unknown","v":0.9},{"v":0.3},{"k":"TongueOut"}]}"#;
        let frame = parse_vts(json).unwrap();
        assert_eq!(frame.shapes_seen, 4);
        assert_eq!(weight(&frame, ArkitShape::EyeBlinkLeft), 0.5);
        assert_eq!(weight(&frame, ArkitShape::JawOpen), 1.0);
        assert_eq!(weight(&frame, ArkitShape::MouthSmileRight), 0.0);
        assert_eq!(weight(&frame, ArkitShape::TongueOut), 0.0);
    }

    #[test]
    fn vts_rejects_malformed_and_faceless_packets() {
        for bad in [
            "",
            "{",
            "[]",
            "null",
            r#"{"FaceFound":"yes","BlendShapes":[]}"#,
            r#"{"FaceFound":true,"BlendShapes":{"k":"JawOpen"}}"#,
            r#"{"FaceFound":true,"BlendShapes":[]}"#,
            r#"{"FaceFound":false,"BlendShapes":[{"k":"JawOpen","v":0.5}]}"#,
            r#"{"BlendShapes":[{"k":"JawOpen","v":0.5}]}"#,
            r#"{"FaceFound":true,"BlendShapes":[{"k":"JawOpen","v":"0.5"}]}"#,
        ] {
            assert!(parse_vts(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn vts_request_matches_the_documented_shape() {
        let request: serde_json::Value = serde_json::from_str(&vts_request(21412)).unwrap();
        assert_eq!(request["messageType"], "iOSTrackingDataRequest");
        assert_eq!(request["ports"][0], 21412);
        assert_eq!(request["sentBy"], "Visage");
        let time = request["time"].as_f64().unwrap();
        assert!((0.5..=10.0).contains(&time));
    }

    #[test]
    fn stale_phone_data_resets_to_neutral() {
        let args = PhoneArgs {
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            protocol: PhoneProtocol::Vts,
            phone_port: None,
            listen: None,
        };
        let mut receiver = PhoneReceiver::new(args);
        let mut data = UnifiedTrackingData::default();
        let frame = parse_ifacialmocap("jawOpen-80").unwrap();
        let t0 = Instant::now();
        receiver.last = Some((frame, t0));
        assert!(receiver.apply(&mut data, t0));
        assert!((data.getu(UnifiedExpressions::JawOpen) - 0.8).abs() < 1e-6);
        assert!(!receiver.apply(&mut data, t0 + STALE * 2));
        assert_eq!(data.getu(UnifiedExpressions::JawOpen), 0.0);
        data.shapes.setu(UnifiedExpressions::JawOpen, 0.4);
        assert!(!receiver.apply(&mut data, t0 + STALE * 3));
        assert_eq!(data.getu(UnifiedExpressions::JawOpen), 0.4);
    }

    #[test]
    fn cli_defaults() {
        use clap::Parser;
        #[derive(clap::Parser)]
        struct Cli {
            #[command(flatten)]
            phone: PhoneArgs,
        }
        let cli = Cli::try_parse_from(["t", "192.168.1.20"]).unwrap();
        assert_eq!(cli.phone.protocol, PhoneProtocol::IFacialMocap);
        assert_eq!(cli.phone.protocol.default_port(), 49983);
        let cli = Cli::try_parse_from([
            "t",
            "10.0.0.2",
            "--protocol",
            "meowface",
            "--listen",
            "21500",
        ])
        .unwrap();
        assert_eq!(cli.phone.protocol, PhoneProtocol::Vts);
        assert_eq!(cli.phone.listen, Some(21500));
        assert!(Cli::try_parse_from(["t", "not-an-ip"]).is_err());
        assert!(Cli::try_parse_from(["t", "10.0.0.2", "--listen", "0"]).is_err());
    }
}
