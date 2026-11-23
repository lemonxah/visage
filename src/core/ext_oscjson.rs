use log::{debug, info, warn};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use rosc::{OscBundle, OscType};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

use super::{bundle::AvatarBundle, folders::config_file};

const OSCJSON_SERVICE: &str = "_oscjson._tcp.local.";

pub struct ExtOscJson {
    _mdns: Option<ServiceDaemon>,
    mdns_recv: Option<mdns_sd::Receiver<ServiceEvent>>,
    oscjson_addr: Option<Arc<str>>,
    next_run: Instant,
    client: reqwest::blocking::Client,
    pending: Option<mpsc::Receiver<Option<OscJsonNode>>>,
}

impl ExtOscJson {
    pub fn new() -> Self {
        let (mdns, mdns_recv) = match ServiceDaemon::new()
            .and_then(|mdns| mdns.browse(OSCJSON_SERVICE).map(|recv| (mdns, recv)))
        {
            Ok((mdns, recv)) => (Some(mdns), Some(recv)),
            Err(e) => {
                warn!("mDNS browsing unavailable, avatar parameters will not be discovered: {e}");
                (None, None)
            }
        };
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|e| {
                warn!("Falling back to default HTTP client: {e}");
                reqwest::blocking::Client::new()
            });

        Self {
            _mdns: mdns,
            mdns_recv,
            oscjson_addr: None,
            next_run: Instant::now(),
            client,
            pending: None,
        }
    }

    pub fn step(&mut self) -> bool {
        let mut notify_avatar = false;
        let Some(recv) = self.mdns_recv.as_ref() else {
            return notify_avatar;
        };
        if self.next_run > Instant::now() {
            return notify_avatar;
        }
        self.next_run = Instant::now() + Duration::from_secs(15);

        for event in recv.try_iter() {
            let ServiceEvent::ServiceResolved(info) = event else {
                continue;
            };
            if !info.get_fullname().starts_with("VRChat-Client-") {
                continue;
            }
            let addresses = info.get_addresses_v4();
            let Some(addr) = addresses
                .iter()
                .find(|a| a.is_loopback())
                .or_else(|| addresses.iter().next())
            else {
                continue;
            };
            info!(
                "Found OSCJSON service: {} @ {}:{}",
                info.get_fullname(),
                addr,
                info.get_port()
            );

            if self.oscjson_addr.is_none() {
                notify_avatar = true;
            }

            self.oscjson_addr = Some(format!("http://{}:{}/avatar", addr, info.get_port()).into());
        }

        notify_avatar
    }

    pub fn request(&mut self, avatar: AvatarIdentifier) {
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        let addr = self.oscjson_addr.clone();
        let client = self.client.clone();
        let spawned = thread::Builder::new()
            .name("avatar-json".into())
            .spawn(move || {
                let _ = tx.send(load_avatar(&avatar, addr.as_deref(), &client));
            });
        if let Err(e) = spawned {
            warn!("Could not start avatar json fetch: {e}");
            self.pending = None;
        }
    }

    pub fn poll(&mut self) -> Option<Option<OscJsonNode>> {
        let rx = self.pending.as_ref()?;
        match rx.try_recv() {
            Ok(node) => {
                self.pending = None;
                Some(node)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                Some(None)
            }
        }
    }
}

fn load_avatar(
    avatar: &AvatarIdentifier,
    addr: Option<&str>,
    client: &reqwest::blocking::Client,
) -> Option<OscJsonNode> {
    let mut json = String::new();

    if let AvatarIdentifier::Path(path) = avatar {
        if let Err(e) = File::open(path).and_then(|mut f| f.read_to_string(&mut json)) {
            log::error!("Could not read file: {:?}", e);
            return None;
        }
    } else {
        let Some(addr) = addr else {
            warn!("No avatar oscjson address.");
            return None;
        };

        thread::sleep(Duration::from_millis(250));

        let resp = match client.get(addr).send() {
            Ok(resp) => resp,
            Err(e) => {
                warn!("Failed to fetch avatar json: {e}");
                return None;
            }
        };

        let Ok(text) = resp.text() else {
            warn!("No payload in avatar json response.");
            return None;
        };

        json = text;
    }

    let path = config_file("avatar.json", None);
    if let Err(e) = File::create(path).and_then(|mut f| f.write_all(json.as_bytes())) {
        warn!("Could not write avatar json file: {:?}", e);
    }

    match serde_json::from_str(&json) {
        Ok(root_node) => Some(root_node),
        Err(e) => {
            warn!("Failed to deserialize avatar json: {}\n", e);
            debug!("{}", json);
            None
        }
    }
}

#[derive(Debug)]
pub enum AvatarIdentifier {
    Default,
    Uid(String),
    Path(String),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OscJsonNode {
    #[serde(alias = "FULL_PATH")]
    pub full_path: Arc<str>,
    #[serde(alias = "ACCESS")]
    pub access: i32,
    #[serde(alias = "TYPE")]
    pub data_type: Option<Arc<str>>,
    #[serde(alias = "CONTENTS")]
    pub contents: Option<HashMap<Arc<str>, OscJsonNode>>,
    #[serde(alias = "VALUE")]
    pub value: Option<serde_json::Value>,
}

impl OscJsonNode {
    pub fn get(&self, path: &str) -> Option<&OscJsonNode> {
        let mut node = self;
        for part in path.split('/') {
            if let Some(contents) = &node.contents {
                node = contents.get(part)?;
            } else {
                return None;
            }
        }
        Some(node)
    }

    pub fn has_vsync(&self) -> bool {
        self.get("parameters")
            .and_then(|parameters| parameters.get("VSync"))
            .is_some()
    }

    /// Whether the avatar has Face Tracking with Use All Tracking Keys, if VRC reported it
    pub fn full_face_tracking(&self) -> Option<bool> {
        self.get("avatarfullfacetracking")?
            .value
            .as_ref()?
            .get(0)?
            .as_bool()
    }
}

#[derive(Clone)]
pub struct MysteryParam {
    pub name: Arc<str>,
    pub main_address: Option<Arc<str>>,
    pub addresses: [Option<Arc<str>>; 7],
    pub neg_address: Option<Arc<str>>,
    pub num_bits: usize,
    pub last_value: f32,
    pub last_bits: [bool; 8],
}

impl MysteryParam {
    pub fn send(&mut self, value: f32, bundle: &mut OscBundle) {
        if let Some(addr) = self.main_address.as_ref() {
            if (value - self.last_value).abs() > 0.01 {
                bundle.send_parameter(addr, OscType::Float(value));
                self.last_value = value;
            }
        }

        let mut value = value;
        if let Some(addr) = self.neg_address.as_ref() {
            let send_val = value < 0.;
            if self.last_bits[7] != send_val {
                bundle.send_parameter(addr, OscType::Bool(send_val));
                self.last_bits[7] = send_val;
            }
            value = value.abs();
        } else if value < 0. {
            value = 0.;
        }

        let value = (value * ((1 << self.num_bits) - 1) as f32) as i32;

        self.addresses
            .iter()
            .enumerate()
            .take(self.num_bits)
            .for_each(|(idx, param)| {
                if let Some(addr) = param.as_ref() {
                    let send_val = value & (1 << idx) != 0;
                    if self.last_bits[idx] != send_val {
                        bundle.send_parameter(addr, OscType::Bool(send_val));
                        self.last_bits[idx] = send_val;
                    }
                }
            });
    }
}
