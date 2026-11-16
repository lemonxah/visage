use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        mpsc::{sync_channel, Receiver, SyncSender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use colored::{Color, Colorize};
use once_cell::sync::Lazy;
use rosc::{OscPacket, OscType};

use crate::core::{
    ext_tracking::unified::UnifiedExpressions, AppState, INSTRUCTIONS_END, INSTRUCTIONS_START,
    TRACK_ON,
};

use super::{
    unified::{UnifiedTrackingData, NUM_SHAPES},
    FaceReceiver,
};

const MOUTH_TIMEOUT: Duration = Duration::from_secs(1);

static STA_BABL1: Lazy<Arc<str>> = Lazy::new(|| format!("{}", "BABBLE".color(Color::Green)).into());
static STA_BABL0: Lazy<Arc<str>> = Lazy::new(|| format!("{}", "BABBLE".color(Color::Red)).into());
static STA_ETVR1: Lazy<Arc<str>> = Lazy::new(|| format!("{}", "ETVR".color(Color::Green)).into());
static STA_ETVR0: Lazy<Arc<str>> = Lazy::new(|| format!("{}", "ETVR".color(Color::Red)).into());

pub(super) struct BabbleEtvrReceiver {
    listen_port: u16,
    sender: SyncSender<Box<BabbleEtvrEvent>>,
    receiver: Receiver<Box<BabbleEtvrEvent>>,
    last_received_babble: Instant,
    last_received_etvr: Instant,
    mouth_values: [Option<(f32, Instant)>; NUM_SHAPES],
}

impl BabbleEtvrReceiver {
    pub fn new(listen_port: u16) -> Self {
        let (sender, receiver) = sync_channel(128);
        Self {
            listen_port,
            sender,
            receiver,
            last_received_babble: Instant::now(),
            last_received_etvr: Instant::now(),
            mouth_values: [None; NUM_SHAPES],
        }
    }

    pub(super) fn start_mouth_loop(&mut self) {
        log::info!("Using OpenXR gaze, blinks and brows with Babble mouth tracking.");
        log::info!(
            "Babble OSC output: 127.0.0.1:{}, empty prefix/location.",
            self.listen_port
        );
        log::info!("BABBLE turns green when mouth data arrives; stale values fall back to OpenXR after one second.");
        let sender = self.sender.clone();
        let port = self.listen_port;
        thread::spawn(move || babble_loop(port, sender));
    }

    fn apply_mouth(&mut self, data: &mut UnifiedTrackingData) -> bool {
        let now = Instant::now();
        for event in self.receiver.try_iter() {
            // Preserve all OpenXR eye, brow and upper-cheek values even if an
            // eye tracker is sending to the same port as Babble.
            if (event.expression as usize) >= UnifiedExpressions::CheekPuffRight as usize {
                self.mouth_values[event.expression as usize] = Some((event.value, now));
            }
        }
        let mut fresh = false;
        for (index, cached) in self.mouth_values.iter_mut().enumerate() {
            if let Some((value, at)) = *cached {
                if now.duration_since(at) < MOUTH_TIMEOUT {
                    data.shapes[index] = value;
                    fresh = true;
                } else {
                    *cached = None;
                }
            }
        }
        fresh
    }

    pub(super) fn receive_mouth(&mut self, data: &mut UnifiedTrackingData, state: &mut AppState) {
        let fresh = self.apply_mouth(data);
        state.status.add_item(if fresh {
            STA_BABL1.clone()
        } else {
            STA_BABL0.clone()
        });
    }
}

impl FaceReceiver for BabbleEtvrReceiver {
    fn start_loop(&mut self) {
        let sender = self.sender.clone();
        let listen_port = self.listen_port;

        let babble_recv_port = listen_port + 10;
        let babble_http_port = babble_recv_port + 1;
        let etvr_recv_port = babble_recv_port + 10;
        let etvr_http_port = etvr_recv_port + 1;

        log::info!("{}", *INSTRUCTIONS_START);
        log::info!("");
        log::info!("Selected ProjectBabble + EyeTrackVR to provide face data.");
        log::info!("(You don't have to have both!)");
        log::info!("");
        log::info!("For Babble:");
        log::info!(
            "• Set {} to {}",
            "Port".color(Color::BrightYellow),
            format!("{}", listen_port).color(Color::Cyan),
        );
        log::info!(
            "• Set {} to {}",
            "Receiver Port".color(Color::BrightYellow),
            format!("{}", babble_recv_port).color(Color::Cyan),
        );
        log::info!(
            "• Start: {}",
            format!(
                "./VrcAdvert babble {} {}",
                babble_http_port, babble_recv_port
            )
            .on_color(Color::White)
            .color(Color::Black)
        );
        log::info!("");
        log::info!("For ETVR:");
        log::info!(
            "• Set {} to {}",
            "OSC Port".color(Color::BrightYellow),
            format!("{}", listen_port).color(Color::Cyan),
        );
        log::info!(
            "• Set {} to {}",
            "OSC Receiver Port".color(Color::BrightYellow),
            format!("{}", etvr_recv_port).color(Color::Cyan),
        );
        log::info!(
            "• Start: {}",
            format!("./VrcAdvert etvr {} {}", etvr_http_port, etvr_recv_port)
                .on_color(Color::White)
                .color(Color::Black)
        );
        log::info!("");
        log::info!("Status bar tickers:");
        log::info!("• {} → mouth data is being received", *STA_BABL1);
        log::info!("• {} → eye data is being received", *STA_ETVR1);
        log::info!(
            "• {} → head & wrist data is being received (for AutoPilot)",
            *TRACK_ON
        );
        log::info!("");
        log::info!("To use AutoPilot:");
        log::info!("• In VRChat Settings/Tracking & IK: enable sending of Head and Wrist data");
        log::info!("");
        log::info!("{}", *INSTRUCTIONS_END);

        thread::spawn(move || babble_loop(listen_port, sender));
    }

    fn receive(&mut self, data: &mut UnifiedTrackingData, state: &mut AppState) {
        for event in self.receiver.try_iter() {
            data.shapes[event.expression as usize] = event.value;

            if (event.expression as usize) < (UnifiedExpressions::BrowPinchRight as usize) {
                self.last_received_etvr = Instant::now();
            } else {
                self.last_received_babble = Instant::now();
            }
        }

        if self.last_received_babble.elapsed() < Duration::from_secs(1) {
            state.status.add_item(STA_BABL1.clone());
        } else {
            state.status.add_item(STA_BABL0.clone());
        }

        if self.last_received_etvr.elapsed() < Duration::from_secs(1) {
            state.status.add_item(STA_ETVR1.clone());
        } else {
            state.status.add_item(STA_ETVR0.clone());
        }
    }
}

fn babble_loop(listen_port: u16, mut sender: SyncSender<Box<BabbleEtvrEvent>>) {
    loop {
        if let Some(()) = receive_babble_osc(listen_port, &mut sender) {
            break;
        } else {
            thread::sleep(Duration::from_millis(5000));
        }
    }
}

fn receive_babble_osc(
    listen_port: u16,
    sender: &mut SyncSender<Box<BabbleEtvrEvent>>,
) -> Option<()> {
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener = UdpSocket::bind(SocketAddr::new(ip, listen_port)).expect("bind listener socket");
    let mut buf = [0u8; 65535];
    loop {
        if let Ok((size, _addr)) = listener.recv_from(&mut buf) {
            if let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..size]) {
                queue_packet(packet, sender);
            }
        }
    }
}

fn queue_packet(packet: OscPacket, sender: &SyncSender<Box<BabbleEtvrEvent>>) {
    let packet = match packet {
        OscPacket::Bundle(bundle) => {
            for packet in bundle.content {
                queue_packet(packet, sender);
            }
            return;
        }
        OscPacket::Message(packet) => packet,
    };
    let Some(OscType::Float(value)) = packet.args.first() else {
        return;
    };
    if !value.is_finite() {
        return;
    }
    if let Some(mappings) = ADDR_TO_UNIFIED.get(packet.addr.as_str()) {
        for mapping in mappings {
            let value = mapping.value_mapper.map_or(*value, |mapper| mapper(*value));
            let event = Box::new(BabbleEtvrEvent::new(mapping.expression, value));
            if let Err(error) = sender.try_send(event) {
                log::warn!("Failed to send Babble/ETVR message: {error}");
            }
        }
    }
}

struct BabbleEtvrEvent {
    pub expression: UnifiedExpressions,
    pub value: f32,
}

impl BabbleEtvrEvent {
    pub fn new(expression: UnifiedExpressions, value: f32) -> Self {
        Self { expression, value }
    }
}

#[derive(Debug, Clone)]
struct ExpressionMapping {
    pub expression: UnifiedExpressions,
    pub value_mapper: Option<fn(f32) -> f32>,
}

fn mapper_invert_normalized(x: f32) -> f32 {
    1.0 - x
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::vec3;
    use rosc::OscMessage;

    fn message(address: &str, value: f32) -> OscPacket {
        OscPacket::Message(OscMessage {
            addr: address.into(),
            args: vec![OscType::Float(value)],
        })
    }

    #[test]
    fn mouth_overlay_preserves_openxr_gaze_blinks_and_brows() {
        let mut receiver = BabbleEtvrReceiver::new(9400);
        let mut native = UnifiedTrackingData::default();
        native.eyes = [Some(vec3(0.1, -0.2, 0.0)), Some(vec3(0.1, 0.2, 0.0))];
        native.setu(UnifiedExpressions::EyeClosedLeft, 0.4);
        native.setu(UnifiedExpressions::BrowInnerUpLeft, 0.6);
        native.setu(UnifiedExpressions::JawOpen, 0.2);
        let mut merged = native.clone();
        for packet in [
            message("/jawOpen", 0.9),
            message("/tongueOut", 0.7),
            message("/LeftEyeX", 1.0),
            message("/LeftEyeLid", 0.0),
        ] {
            queue_packet(packet, &receiver.sender);
        }
        assert!(receiver.apply_mouth(&mut merged));
        assert_eq!(merged.eyes, native.eyes);
        assert_eq!(merged.getu(UnifiedExpressions::EyeLeftX), 0.0);
        assert_eq!(merged.getu(UnifiedExpressions::EyeClosedLeft), 0.4);
        assert_eq!(merged.getu(UnifiedExpressions::BrowInnerUpLeft), 0.6);
        assert_eq!(merged.getu(UnifiedExpressions::JawOpen), 0.9);
        assert_eq!(merged.getu(UnifiedExpressions::TongueOut), 0.7);

        // Cached Babble mouth values survive the next OpenXR frame.
        let mut next = native.clone();
        assert!(receiver.apply_mouth(&mut next));
        assert_eq!(next.getu(UnifiedExpressions::JawOpen), 0.9);

        // Losing Babble restores the native mouth, including a zero tongue.
        for value in receiver.mouth_values.iter_mut().flatten() {
            value.1 = Instant::now() - MOUTH_TIMEOUT * 2;
        }
        let mut expired = native.clone();
        assert!(!receiver.apply_mouth(&mut expired));
        assert_eq!(expired.shapes, native.shapes);
        assert_eq!(expired.getu(UnifiedExpressions::TongueOut), 0.0);
    }

    #[test]
    fn nested_babble_bundles_map_all_mouth_shapes() {
        let mut receiver = BabbleEtvrReceiver::new(9400);
        let packet = OscPacket::Bundle(rosc::OscBundle {
            timetag: (0, 1).into(),
            content: vec![
                message("/jawOpen", 0.8),
                OscPacket::Bundle(rosc::OscBundle {
                    timetag: (0, 1).into(),
                    content: vec![message("/mouthSmileLeft", 0.7)],
                }),
            ],
        });
        let bytes = rosc::encoder::encode(&packet).unwrap();
        let (_, decoded) = rosc::decoder::decode_udp(&bytes).unwrap();
        queue_packet(decoded, &receiver.sender);
        let mut data = UnifiedTrackingData::default();
        assert!(receiver.apply_mouth(&mut data));
        assert_eq!(data.getu(UnifiedExpressions::JawOpen), 0.8);
        assert_eq!(data.getu(UnifiedExpressions::MouthCornerPullLeft), 0.7);
        assert_eq!(data.getu(UnifiedExpressions::MouthCornerSlantLeft), 0.7);
    }

    #[test]
    fn missing_or_invalid_babble_does_not_override_the_native_mouth() {
        let mut receiver = BabbleEtvrReceiver::new(9400);
        let mut data = UnifiedTrackingData::default();
        data.setu(UnifiedExpressions::JawOpen, 0.3);
        queue_packet(message("/jawOpen", f32::NAN), &receiver.sender);
        queue_packet(message("/unrecognized", 1.0), &receiver.sender);
        assert!(!receiver.apply_mouth(&mut data));
        assert_eq!(data.getu(UnifiedExpressions::JawOpen), 0.3);
    }
}

#[rustfmt::skip]
static ADDR_TO_UNIFIED: Lazy<HashMap<&'static str, Vec<ExpressionMapping>>> = Lazy::new(|| {
    [
        // ProjectBabble
        ("/cheekPuffLeft", vec![ExpressionMapping{expression: UnifiedExpressions::CheekPuffLeft, value_mapper: None}]),
        ("/cheekPuffRight", vec![ExpressionMapping{expression: UnifiedExpressions::CheekPuffRight, value_mapper: None}]),
        ("/cheekSuckLeft", vec![ExpressionMapping{expression: UnifiedExpressions::CheekSuckLeft, value_mapper: None}]),
        ("/cheekSuckRight", vec![ExpressionMapping{expression: UnifiedExpressions::CheekSuckRight, value_mapper: None}]),
        ("/jawOpen", vec![ExpressionMapping{expression: UnifiedExpressions::JawOpen, value_mapper: None}]),
        ("/jawForward", vec![ExpressionMapping{expression: UnifiedExpressions::JawForward, value_mapper: None}]),
        ("/jawLeft", vec![ExpressionMapping{expression: UnifiedExpressions::JawLeft, value_mapper: None}]),
        ("/jawRight", vec![ExpressionMapping{expression: UnifiedExpressions::JawRight, value_mapper: None}]),
        ("/noseSneerLeft", vec![ExpressionMapping{expression: UnifiedExpressions::NoseSneerLeft, value_mapper: None}]),
        ("/noseSneerRight", vec![ExpressionMapping{expression: UnifiedExpressions::NoseSneerRight, value_mapper: None}]),
        ("/mouthFunnel", vec![ExpressionMapping{expression: UnifiedExpressions::LipFunnelUpperRight, value_mapper: None}, ExpressionMapping{expression: UnifiedExpressions::LipFunnelUpperLeft, value_mapper: None}, ExpressionMapping{expression: UnifiedExpressions::LipFunnelLowerRight, value_mapper: None}, ExpressionMapping{expression: UnifiedExpressions::LipFunnelLowerLeft, value_mapper: None}]),
        ("/mouthPucker", vec![ExpressionMapping{expression: UnifiedExpressions::LipPuckerUpperRight, value_mapper: None}, ExpressionMapping{expression: UnifiedExpressions::LipPuckerUpperLeft, value_mapper: None}, ExpressionMapping{expression: UnifiedExpressions::LipPuckerLowerRight, value_mapper: None}, ExpressionMapping{expression: UnifiedExpressions::LipPuckerLowerLeft, value_mapper: None}]),
        ("/mouthLeft", vec![ExpressionMapping{expression: UnifiedExpressions::MouthPressLeft, value_mapper: None}]),
        ("/mouthRight", vec![ExpressionMapping{expression: UnifiedExpressions::MouthPressRight, value_mapper: None}]),
        ("/mouthRollUpper", vec![ExpressionMapping{expression: UnifiedExpressions::LipSuckUpperLeft, value_mapper: None}, ExpressionMapping{expression: UnifiedExpressions::LipSuckUpperRight, value_mapper: None}]),
        ("/mouthRollLower", vec![ExpressionMapping{expression: UnifiedExpressions::LipSuckLowerLeft, value_mapper: None}, ExpressionMapping{expression: UnifiedExpressions::LipSuckLowerRight, value_mapper: None}]),
        ("/mouthShrugUpper", vec![ExpressionMapping{expression: UnifiedExpressions::MouthRaiserUpper, value_mapper: None}]),
        ("/mouthShrugLower", vec![ExpressionMapping{expression: UnifiedExpressions::MouthRaiserLower, value_mapper: None}]),
        ("/mouthClose", vec![ExpressionMapping{expression: UnifiedExpressions::MouthClosed, value_mapper: None}]),
        ("/mouthSmileLeft", vec![ExpressionMapping{expression: UnifiedExpressions::MouthCornerPullLeft, value_mapper: None}, ExpressionMapping{expression: UnifiedExpressions::MouthCornerSlantLeft, value_mapper: None}]),
        ("/mouthSmileRight", vec![ExpressionMapping{expression: UnifiedExpressions::MouthCornerPullRight, value_mapper: None}, ExpressionMapping{expression: UnifiedExpressions::MouthCornerSlantRight, value_mapper: None}]),
        ("/mouthFrownLeft", vec![ExpressionMapping{expression: UnifiedExpressions::MouthFrownLeft, value_mapper: None}, ExpressionMapping{expression: UnifiedExpressions::MouthStretchLeft, value_mapper: None}]),
        ("/mouthFrownRight", vec![ExpressionMapping{expression: UnifiedExpressions::MouthFrownRight, value_mapper: None}, ExpressionMapping{expression: UnifiedExpressions::MouthStretchRight, value_mapper: None}]),
        ("/mouthDimpleLeft", vec![ExpressionMapping{expression: UnifiedExpressions::MouthDimpleLeft, value_mapper: None}]),
        ("/mouthDimpleRight", vec![ExpressionMapping{expression: UnifiedExpressions::MouthDimpleRight, value_mapper: None}]),
        ("/mouthUpperUpLeft", vec![ExpressionMapping{expression: UnifiedExpressions::MouthUpperUpLeft, value_mapper: None}]),
        ("/mouthUpperUpRight", vec![ExpressionMapping{expression: UnifiedExpressions::MouthUpperUpRight, value_mapper: None}]),
        ("/mouthLowerDownLeft", vec![ExpressionMapping{expression: UnifiedExpressions::MouthLowerDownLeft, value_mapper: None}]),
        ("/mouthLowerDownRight", vec![ExpressionMapping{expression: UnifiedExpressions::MouthLowerDownRight, value_mapper: None}]),
        ("/mouthStretchLeft", vec![ExpressionMapping{expression: UnifiedExpressions::MouthStretchLeft, value_mapper: None}]),
        ("/mouthStretchRight", vec![ExpressionMapping{expression: UnifiedExpressions::MouthStretchRight, value_mapper: None}]),
        ("/tongueOut", vec![ExpressionMapping{expression: UnifiedExpressions::TongueOut, value_mapper: None}]),
        ("/tongueUp", vec![ExpressionMapping{expression: UnifiedExpressions::TongueUp, value_mapper: None}]),
        ("/tongueDown", vec![ExpressionMapping{expression: UnifiedExpressions::TongueDown, value_mapper: None}]),
        ("/tongueLeft", vec![ExpressionMapping{expression: UnifiedExpressions::TongueLeft, value_mapper: None}]),
        ("/tongueRight", vec![ExpressionMapping{expression: UnifiedExpressions::TongueRight, value_mapper: None}]),
        ("/tongueRoll", vec![ExpressionMapping{expression: UnifiedExpressions::TongueRoll, value_mapper: None}]),
        ("/tongueBendDown", vec![ExpressionMapping{expression: UnifiedExpressions::TongueBendDown, value_mapper: None}]),
        ("/tongueCurlUp", vec![ExpressionMapping{expression: UnifiedExpressions::TongueCurlUp, value_mapper: None}]),
        ("/tongueSquish", vec![ExpressionMapping{expression: UnifiedExpressions::TongueSquish, value_mapper: None}]),
        ("/tongueFlat", vec![ExpressionMapping{expression: UnifiedExpressions::TongueFlat, value_mapper: None}]),
        ("/tongueTwistLeft", vec![ExpressionMapping{expression: UnifiedExpressions::TongueTwistLeft, value_mapper: None}]),
        ("/tongueTwistRight", vec![ExpressionMapping{expression: UnifiedExpressions::TongueTwistRight, value_mapper: None}]),
        ("/mouthPressLeft", vec![ExpressionMapping{expression: UnifiedExpressions::MouthPressLeft, value_mapper: None}]),
        ("/mouthPressRight", vec![ExpressionMapping{expression: UnifiedExpressions::MouthPressRight, value_mapper: None}]),

        // ProjectBabble Baballonia Eye Tracking
        ("/LeftEyeX", vec![ExpressionMapping{expression: UnifiedExpressions::EyeLeftX, value_mapper: None}]),
        ("/RightEyeX", vec![ExpressionMapping{expression: UnifiedExpressions::EyeRightX, value_mapper: None}]),
        // TODO: ETVR only has one Y value for both eyes, but Babble has separate ones. 
        // For now, only the left eye Y value is used for simplicity, 
        // but maybe in the future we could do some kind of averaging or something to use both?
        // if there's demand for it.
        ("/LeftEyeY", vec![ExpressionMapping{expression: UnifiedExpressions::EyeY, value_mapper: None}]),
        // ("/RightEyeY", vec![MappingData{expression: UnifiedExpressions::EyeY, value_mapper: None}]), 
        ("/LeftEyeLid", vec![ExpressionMapping{expression: UnifiedExpressions::EyeClosedLeft, value_mapper: Some(mapper_invert_normalized)}]),
        ("/RightEyeLid", vec![ExpressionMapping{expression: UnifiedExpressions::EyeClosedRight, value_mapper: Some(mapper_invert_normalized)}]),

        // ETVR
        ("/avatar/parameters/LeftEyeX", vec![ExpressionMapping{expression: UnifiedExpressions::EyeLeftX, value_mapper: None}]),
        ("/avatar/parameters/RightEyeX", vec![ExpressionMapping{expression: UnifiedExpressions::EyeRightX, value_mapper: None}]),
        ("/avatar/parameters/EyesY", vec![ExpressionMapping{expression: UnifiedExpressions::EyeY, value_mapper: None}]),
        ("/avatar/parameters/LeftEyeLid", vec![ExpressionMapping{expression: UnifiedExpressions::EyeClosedLeft, value_mapper: None}]),
        ("/avatar/parameters/RightEyeLid", vec![ExpressionMapping{expression: UnifiedExpressions::EyeClosedRight, value_mapper: None}]),

        ("/avatar/parameters/v2/EyeLeftX", vec![ExpressionMapping{expression: UnifiedExpressions::EyeLeftX, value_mapper: None}]),
        ("/avatar/parameters/v2/EyeRightX", vec![ExpressionMapping{expression: UnifiedExpressions::EyeRightX, value_mapper: None}]),
        ("/avatar/parameters/v2/EyeLeftY", vec![ExpressionMapping{expression: UnifiedExpressions::EyeY, value_mapper: None}]),
        ("/avatar/parameters/v2/EyeLidLeft", vec![ExpressionMapping{expression: UnifiedExpressions::EyeClosedLeft, value_mapper: None}]),
        ("/avatar/parameters/v2/EyeLidRight", vec![ExpressionMapping{expression: UnifiedExpressions::EyeClosedRight, value_mapper: None}]),
    ]
    .into_iter()
    .collect()
});
