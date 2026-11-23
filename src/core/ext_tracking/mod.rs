use std::{array, str::FromStr, sync::Arc};

use colored::{Color, Colorize};
use once_cell::sync::Lazy;
use regex::Regex;
use rosc::{OscBundle, OscType};
use sranipal::SRanipalExpression;

use crate::{FaceSetup, FtMode};

#[cfg(feature = "alvr")]
use self::alvr::AlvrReceiver;

#[cfg(feature = "babble")]
use self::babble::BabbleEtvrReceiver;

#[cfg(feature = "openxr")]
pub use self::openxr::OpenXrArgs;
#[cfg(feature = "openxr")]
use self::openxr::OpenXrReceiver;

pub use self::phone::PhoneArgs;

use self::unified::{CombinedExpression, UnifiedExpressions, UnifiedTrackingData, NUM_SHAPES};

use super::{
    ext_oscjson::{MysteryParam, OscJsonNode},
    live::LIVE,
    AppState,
};

use strum::EnumCount;
use strum::IntoEnumIterator;

#[cfg(feature = "alvr")]
mod alvr;
#[cfg(feature = "openxr")]
mod android_face;
mod arkit;
#[cfg(feature = "babble")]
mod babble;
#[cfg(feature = "cameras")]
pub mod camera;
mod face2_fb;
#[cfg(any(feature = "openxr", feature = "alvr"))]
mod htc;
#[cfg(feature = "openxr")]
mod openxr;
mod phone;
#[cfg(feature = "qpro-tongue")]
pub mod qpro_tongue;
mod sranipal;
mod steamlink;
pub mod unified;
#[cfg(feature = "openxr")]
mod vergence;

#[cfg(any(feature = "cameras", feature = "qpro-tongue"))]
pub(crate) fn onnx_environment() -> anyhow::Result<ort::environment::Environment> {
    static ENVIRONMENT: once_cell::sync::OnceCell<ort::environment::Environment> =
        once_cell::sync::OnceCell::new();
    ENVIRONMENT
        .get_or_try_init(|| ort::init().with_name("visage").build())
        .cloned()
        .map_err(|e| anyhow::anyhow!("ONNX Runtime: {e}"))
}

trait FaceReceiver {
    fn start_loop(&mut self);
    fn receive(&mut self, _data: &mut UnifiedTrackingData, _: &mut AppState);
}

static STA_NATIVE: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "NATIVE".color(Color::Green)).into());
static STA_LEGACY: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "LEGACY".color(Color::Yellow)).into());
static STA_BOTH: Lazy<Arc<str>> =
    Lazy::new(|| format!("{}", "NATIVE+LEGACY".color(Color::Cyan)).into());

struct DummyReceiver;

impl FaceReceiver for DummyReceiver {
    fn start_loop(&mut self) {}
    fn receive(&mut self, _data: &mut UnifiedTrackingData, _: &mut AppState) {}
}

pub struct ExtTracking {
    pub data: UnifiedTrackingData,
    params: [Option<MysteryParam>; NUM_SHAPES],
    receiver: Box<dyn FaceReceiver>,
    ft_mode: FtMode,
}

impl ExtTracking {
    pub fn new(setup: FaceSetup, ft_mode: FtMode) -> Self {
        let default_combined = vec![
            CombinedExpression::BrowExpressionLeft,
            CombinedExpression::BrowExpressionRight,
            CombinedExpression::EyeLidLeft,
            CombinedExpression::EyeLidRight,
            CombinedExpression::JawX,
            CombinedExpression::LipFunnelLower,
            CombinedExpression::LipFunnelUpper,
            CombinedExpression::LipPucker,
            CombinedExpression::MouthLowerDown,
            CombinedExpression::MouthStretchTightenLeft,
            CombinedExpression::MouthStretchTightenRight,
            CombinedExpression::MouthUpperUp,
            CombinedExpression::MouthX,
            CombinedExpression::SmileSadLeft,
            CombinedExpression::SmileSadRight,
        ];
        let default_unified = vec![
            UnifiedExpressions::CheekPuffLeft,
            UnifiedExpressions::CheekPuffRight,
            UnifiedExpressions::EyeSquintLeft,
            UnifiedExpressions::EyeSquintRight,
            UnifiedExpressions::JawOpen,
            UnifiedExpressions::MouthClosed,
        ];

        let mut params = array::from_fn(|_| None);

        for e in default_combined.into_iter() {
            let name: &str = e.into();
            let new = MysteryParam {
                name: name.into(),
                main_address: Some(format!("FT/v2/{}", name).into()),
                addresses: array::from_fn(|_| None),
                neg_address: None,
                num_bits: 0,
                last_value: 0.,
                last_bits: [false; 8],
            };
            params[e as usize] = Some(new);
        }

        for e in default_unified.into_iter() {
            let name: &str = e.into();
            let new = MysteryParam {
                name: name.into(),
                main_address: Some(format!("FT/v2/{}", name).into()),
                addresses: array::from_fn(|_| None),
                neg_address: None,
                num_bits: 0,
                last_value: 0.,
                last_bits: [false; 8],
            };
            params[e as usize] = Some(new);
        }

        #[cfg(feature = "cameras")]
        let camera_args = camera::CameraArgs::from_setup(&setup);
        let receiver: Box<dyn FaceReceiver> = match setup {
            FaceSetup::Dummy => Box::new(DummyReceiver {}),
            #[cfg(feature = "alvr")]
            FaceSetup::Alvr => Box::new(AlvrReceiver::new()),
            #[cfg(feature = "openxr")]
            FaceSetup::Openxr(args) => {
                #[cfg(feature = "babble")]
                let receiver: Box<dyn FaceReceiver> = match args.babble {
                    Some(port) => Box::new(openxr::OpenXrBabbleReceiver::new(args, port)),
                    None => Box::new(OpenXrReceiver::new(args)),
                };
                #[cfg(not(feature = "babble"))]
                let receiver: Box<dyn FaceReceiver> = Box::new(OpenXrReceiver::new(args));
                receiver
            }
            #[cfg(feature = "babble")]
            FaceSetup::Babble { listen, .. } => Box::new(BabbleEtvrReceiver::new(listen)),
            FaceSetup::Phone(args) => Box::new(phone::PhoneReceiver::new(args)),
            FaceSetup::Steamlink { listen } => Box::new(steamlink::SteamLinkReceiver::new(listen)),
        };
        #[cfg(feature = "cameras")]
        let receiver = camera::wrap(receiver, camera_args);

        let mut me = Self {
            data: UnifiedTrackingData::default(),
            params,
            receiver,
            ft_mode,
        };

        log::info!("Face tracking mode: {:?}", ft_mode);
        log::info!("--- Default params ---");
        me.print_params();

        me.receiver.start_loop();

        me
    }

    pub fn step(&mut self, state: &mut AppState, bundle: &mut OscBundle) {
        let motion = matches!(state.params.get("Motion"), Some(OscType::Int(1)));
        let face_override = matches!(state.params.get("FaceFreeze"), Some(OscType::Bool(true)));
        let afk = matches!(state.params.get("AFK"), Some(OscType::Bool(true)))
            || matches!(state.params.get("IsAfk"), Some(OscType::Bool(true)));

        let live = LIVE.enabled();
        let overrides = live.then(|| {
            let mut controls = LIVE.controls.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(mode) = controls.ft_mode.take() {
                if mode != self.ft_mode {
                    log::info!("Face tracking mode: {:?}", mode);
                    self.ft_mode = mode;
                }
            }
            controls.overrides
        });

        if afk {
            log::debug!("AFK");
        } else if motion ^ face_override {
            log::debug!("Freeze");
        } else {
            self.receiver.receive(&mut self.data, state);
            if let Some(overrides) = overrides.as_ref() {
                for (shape, value) in self.data.shapes[..UnifiedExpressions::COUNT]
                    .iter_mut()
                    .zip(overrides)
                {
                    if let Some(value) = value {
                        *shape = *value;
                    }
                }
            }
            self.data.calc_combined(state);
        }

        if matches!(state.params.get("FacePause"), Some(OscType::Bool(true))) {
            log::debug!("FacePause");
            return;
        }

        let (native, legacy) = match self.ft_mode {
            FtMode::Auto => (state.avatar_native_ft, !state.avatar_native_ft),
            FtMode::Legacy => (false, true),
            FtMode::Native => (true, false),
            FtMode::Both => (true, true),
        };

        if live {
            let mut snapshot = LIVE.snapshot.lock().unwrap_or_else(|e| e.into_inner());
            snapshot.shapes = self.data.shapes;
            snapshot.eyes = self.data.eyes;
            snapshot.ft_mode = self.ft_mode;
            snapshot.sending_native = native;
            snapshot.sending_legacy = legacy;
            snapshot.avatar_native_ft = state.avatar_native_ft;
        }

        state.status.add_item(match (native, legacy) {
            (true, true) => STA_BOTH.clone(),
            (true, false) => STA_NATIVE.clone(),
            _ => STA_LEGACY.clone(),
        });

        self.data
            .apply_to_bundle(&mut self.params, bundle, native, legacy);
    }

    pub fn osc_json(&mut self, avatar_node: &OscJsonNode) {
        self.params.iter_mut().for_each(|p| *p = None);

        let Some(parameters) = avatar_node.get("parameters") else {
            log::warn!("oscjson: Could not read /avatar/parameters");
            return;
        };

        self.process_node_recursive("parameters", parameters);
        self.print_params();
    }

    fn should_ignore_path(&self, node: &OscJsonNode) -> bool {
        let ignored_paths = vec!["OSCm"];
        let full_path = node.full_path.as_ref();
        for ignored_path in ignored_paths {
            if full_path.contains(ignored_path) {
                return true;
            }
        }
        false
    }

    fn process_node_recursive(&mut self, name: &str, node: &OscJsonNode) -> Option<()> {
        static FT_PARAMS_REGEX: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"^(.+?)(Negative|[0-9]+)?$").unwrap());

        if self.should_ignore_path(node) {
            return None;
        }

        if let Some(contents) = node.contents.as_ref() {
            log::debug!("Checking {}", name);
            for (name, node) in contents.iter() {
                let _ = self.process_node_recursive(name, node);
            }
            return None;
        }

        if let Some(m) = FT_PARAMS_REGEX.captures(name) {
            let main: Arc<str> = m[1].into();

            log::debug!("Param: {}", name);
            let idx = UnifiedExpressions::from_str(&main)
                .map(|e| e as usize)
                .or_else(|_| CombinedExpression::from_str(&main).map(|e| e as usize))
                .or_else(|_| SRanipalExpression::from_str(&main).map(|e| e as usize))
                .ok()?;

            log::debug!(
                "Match: {}",
                UnifiedExpressions::iter()
                    .nth(idx)
                    .map(|e| format!("UnifiedExpressions::{:?}", e))
                    .or_else(|| CombinedExpression::iter()
                        .nth(idx - UnifiedExpressions::COUNT)
                        .map(|e| format!("CombinedExpression::{:?}", e)))
                    .or_else(|| Some("None".to_string()))
                    .unwrap()
            );

            let create = self.params[idx].is_none();

            if create {
                let new = MysteryParam {
                    name: main.clone(),
                    main_address: None,
                    addresses: array::from_fn(|_| None),
                    neg_address: None,
                    num_bits: 0,
                    last_value: 0.,
                    last_bits: [false; 8],
                };
                self.params[idx] = Some(new);
            };

            let stored = self.params[idx].as_mut().unwrap();
            match m.get(2).map(|s| s.as_str()) {
                Some("Negative") => {
                    let addr = &node.full_path.as_ref()[super::PARAM_PREFIX.len()..];
                    stored.neg_address = Some(addr.into());
                }
                Some(digit) => {
                    let Some(idx) = digit
                        .parse::<u32>()
                        .ok()
                        .filter(|d| d.is_power_of_two())
                        .map(|d| d.trailing_zeros() as usize)
                        .filter(|i| *i < stored.addresses.len())
                    else {
                        log::debug!("Ignoring unsupported binary parameter {}", node.full_path);
                        return None;
                    };
                    let addr = &node.full_path.as_ref()[super::PARAM_PREFIX.len()..];
                    stored.num_bits = stored.num_bits.max(idx + 1);
                    stored.addresses[idx] = Some(addr.into());
                }
                None => {
                    let addr = &node.full_path.as_ref()[super::PARAM_PREFIX.len()..];
                    stored.main_address = Some(addr.into());
                }
            }
        }
        None
    }

    fn print_params(&self) {
        for v in self.params.iter().filter_map(|p| p.as_ref()) {
            let mut elems = vec![];

            if v.main_address.is_some() {
                elems.push("float".into())
            }
            if v.num_bits > 0 {
                elems.push(if v.num_bits > 1 {
                    format!("{} bit", v.num_bits)
                } else {
                    format!("{} bits", v.num_bits)
                });
            }
            if v.neg_address.is_some() {
                elems.push("neg".into());
            }
            log::info!("{}: {}", v.name, elems.join(" + "))
        }
    }
}
