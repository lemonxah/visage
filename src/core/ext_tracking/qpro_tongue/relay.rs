use std::{
    io::Read,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use once_cell::sync::Lazy;

use super::stream::FrameSlot;

const START_TIMEOUT: Duration = Duration::from_secs(90);
const STOP_TIMEOUT: Duration = Duration::from_secs(20);
const PIPE_GRACE: Duration = Duration::from_secs(2);
const WATCH_INTERVAL: Duration = Duration::from_secs(2);
const STALL_AFTER: Duration = Duration::from_secs(10);
const RETRY_BASE: Duration = Duration::from_secs(10);
const RETRY_MAX: Duration = Duration::from_secs(60);
const PROBE_INTERVAL: Duration = Duration::from_secs(4);
const ROOT_RECHECK: Duration = Duration::from_secs(30);
const ADB_TIMEOUT: Duration = Duration::from_secs(5);
const QUEST_PRO_DEVICE: &str = "seacliff";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Headset {
    #[default]
    Unknown,
    NoAdb,
    NoDevice,
    Unauthorized,
    NotQuestPro(String),
    NotRooted,
    Ready,
}

impl Headset {
    pub fn ready(&self) -> bool {
        *self == Headset::Ready
    }

    pub fn describe(&self) -> String {
        match self {
            Headset::Unknown => "Checking for a headset…".into(),
            Headset::NoAdb => "adb is not installed or not on PATH.".into(),
            Headset::NoDevice => "No headset connected over adb. Plug it in over USB.".into(),
            Headset::Unauthorized => {
                "The headset has not authorized this PC. Accept the USB debugging prompt in the headset."
                    .into()
            }
            Headset::NotQuestPro(model) => format!(
                "Connected headset is {model}, not a Quest Pro. The mouth cameras need a rooted Quest Pro."
            ),
            Headset::NotRooted => {
                "Quest Pro found, but adb shell has no root. Root it (Singularity or Magisk) and grant Shell root."
                    .into()
            }
            Headset::Ready => "Rooted Quest Pro connected.".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub ok: bool,
    pub output: String,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub script: Option<PathBuf>,
    pub missing: Option<PathBuf>,
    pub busy: bool,
    pub watching: bool,
    pub retry_in: Option<Duration>,
    pub last: Option<Outcome>,
    pub headset: Headset,
}

struct Config {
    script: PathBuf,
    port: u16,
}

#[derive(Default)]
struct Relay {
    config: Option<Config>,
    missing: Option<PathBuf>,
    busy: bool,
    started: bool,
    wanted: bool,
    failures: u32,
    last_attempt: Option<Instant>,
    last_failure: Option<String>,
    last: Option<Outcome>,
    headset: Headset,
}

impl Relay {
    fn backoff(&self) -> Duration {
        RETRY_BASE
            .saturating_mul(1u32 << self.failures.min(6))
            .min(RETRY_MAX)
    }

    fn retry_in(&self) -> Option<Duration> {
        let at = self.last_attempt?;
        Some(self.backoff().saturating_sub(at.elapsed()))
    }
}

static RELAY: Lazy<Mutex<Relay>> = Lazy::new(Mutex::default);

fn relay() -> std::sync::MutexGuard<'static, Relay> {
    RELAY.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn configure(script: Option<&Path>, port: u16, autostart: bool, slot: Arc<FrameSlot>) {
    let Some(script) = script else {
        return;
    };
    match std::fs::canonicalize(script) {
        Ok(script) if script.is_file() => {
            {
                let mut relay = relay();
                relay.config = Some(Config { script, port });
                relay.wanted = autostart;
            }
            spawn_probe();
            spawn_watchdog(slot);
        }
        _ => {
            log::warn!(
                "Tongue: headset relay script not found at {}; the GUI cannot start the cameras",
                script.display()
            );
            relay().missing = Some(script.to_path_buf());
        }
    }
}

pub fn status() -> Status {
    let relay = relay();
    let watching = relay.wanted && relay.config.is_some();
    Status {
        script: relay.config.as_ref().map(|config| config.script.clone()),
        missing: relay.missing.clone(),
        busy: relay.busy,
        watching,
        retry_in: if watching && !relay.busy {
            relay.retry_in()
        } else {
            None
        },
        last: relay.last.clone(),
        headset: relay.headset.clone(),
    }
}

pub fn start() {
    {
        let mut relay = relay();
        if !relay.headset.ready() {
            return;
        }
        relay.wanted = true;
        relay.failures = 0;
        relay.last_attempt = None;
    }
    launch();
}

fn spawn_probe() {
    let spawned = thread::Builder::new()
        .name("tongue-headset-probe".into())
        .spawn(|| {
            let mut rooted: Option<(String, Instant, bool)> = None;
            loop {
                let headset = probe(&mut rooted);
                {
                    let mut relay = relay();
                    if relay.headset != headset {
                        match &headset {
                            Headset::Ready => log::info!("Tongue: {}", headset.describe()),
                            Headset::Unknown => {}
                            other => log::info!("Tongue: {}", other.describe()),
                        }
                        relay.headset = headset;
                    }
                }
                thread::sleep(PROBE_INTERVAL);
            }
        });
    if let Err(e) = spawned {
        log::error!("Tongue: could not start the headset probe: {e}");
    }
}

fn probe(rooted: &mut Option<(String, Instant, bool)>) -> Headset {
    let state = match adb(&["get-state"]) {
        Err(AdbError::Missing) => return Headset::NoAdb,
        Err(AdbError::Failed(output)) => {
            *rooted = None;
            return if output.contains("unauthorized") {
                Headset::Unauthorized
            } else {
                Headset::NoDevice
            };
        }
        Ok(state) => state,
    };
    if state.trim() != "device" {
        *rooted = None;
        return Headset::NoDevice;
    }
    let serial = adb(&["get-serialno"])
        .unwrap_or_default()
        .trim()
        .to_string();
    let device = adb(&["shell", "getprop", "ro.product.device"])
        .unwrap_or_default()
        .trim()
        .to_string();
    if device != QUEST_PRO_DEVICE {
        let model = adb(&["shell", "getprop", "ro.product.model"])
            .unwrap_or_default()
            .trim()
            .to_string();
        if !model.contains("Quest Pro") {
            *rooted = None;
            let name = if model.is_empty() { device } else { model };
            return Headset::NotQuestPro(if name.is_empty() {
                "an unknown device".into()
            } else {
                name
            });
        }
    }
    let recheck = match rooted.as_ref() {
        Some((known, _, true)) if *known == serial => false,
        Some((known, at, false)) if *known == serial => at.elapsed() >= ROOT_RECHECK,
        _ => true,
    };
    if recheck {
        let ok = adb(&["shell", "su -c id"]).is_ok_and(|out| out.contains("uid=0(root)"));
        *rooted = Some((serial, Instant::now(), ok));
    }
    match rooted {
        Some((_, _, true)) => Headset::Ready,
        _ => Headset::NotRooted,
    }
}

enum AdbError {
    Missing,
    Failed(String),
}

fn adb(args: &[&str]) -> Result<String, AdbError> {
    let program = std::env::var("ADB").unwrap_or_else(|_| "adb".into());
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AdbError::Missing,
            _ => AdbError::Failed(e.to_string()),
        })?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + ADB_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                unsafe {
                    libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(_) => break None,
        }
    };
    let text = |receiver: mpsc::Receiver<Vec<u8>>| {
        receiver
            .recv_timeout(PIPE_GRACE)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    };
    let out = text(stdout);
    let err = text(stderr);
    match status {
        Some(status) if status.success() => Ok(out),
        _ => Err(AdbError::Failed(format!("{out}{err}"))),
    }
}

fn spawn_watchdog(slot: Arc<FrameSlot>) {
    let spawned = thread::Builder::new()
        .name("tongue-relay-watch".into())
        .spawn(move || loop {
            thread::sleep(WATCH_INTERVAL);
            let flowing = slot
                .last_frame()
                .is_some_and(|at| at.elapsed() < STALL_AFTER);
            let restart = {
                let mut relay = relay();
                if !flowing && !relay.headset.ready() {
                    false
                } else if flowing {
                    relay.wanted = true;
                    relay.failures = 0;
                    relay.last_failure = None;
                    false
                } else {
                    relay.wanted
                        && !relay.busy
                        && relay.config.is_some()
                        && relay
                            .last_attempt
                            .is_none_or(|at| at.elapsed() >= relay.backoff())
                }
            };
            if restart {
                let silent = slot.last_frame().map(|at| at.elapsed().as_secs());
                match silent {
                    Some(seconds) => log::warn!(
                        "Tongue: no camera frames for {seconds}s; restarting the headset relay"
                    ),
                    None => log::warn!("Tongue: no camera stream yet; retrying the headset relay"),
                }
                if launch() {
                    relay().failures += 1;
                }
            }
        });
    if let Err(e) = spawned {
        log::error!("Tongue: could not start the relay watchdog: {e}");
    }
}

fn launch() -> bool {
    let (script, port) = {
        let mut relay = relay();
        if relay.busy {
            return false;
        }
        let Some(config) = relay.config.as_ref() else {
            return false;
        };
        let job = (config.script.clone(), config.port);
        relay.busy = true;
        relay.last_attempt = Some(Instant::now());
        job
    };

    let spawned = thread::Builder::new()
        .name("tongue-relay".into())
        .spawn(move || {
            log::info!("Tongue: starting headset camera relay");
            let outcome = execute(&script, port, "start", START_TIMEOUT);
            let mut relay = relay();
            let reason = outcome
                .output
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("no output")
                .to_string();
            let repeated = !outcome.ok && relay.last_failure.as_deref() == Some(reason.as_str());
            for line in outcome
                .output
                .lines()
                .filter(|line| !line.trim().is_empty())
            {
                if outcome.ok {
                    log::info!("relay: {line}");
                } else if repeated {
                    log::debug!("relay: {line}");
                } else {
                    log::warn!("relay: {line}");
                }
            }
            if outcome.ok {
                relay.last_failure = None;
            } else {
                if !repeated {
                    log::error!(
                        "Tongue: headset camera relay did not start; it will keep retrying"
                    );
                }
                relay.last_failure = Some(reason);
            }
            relay.busy = false;
            relay.last_attempt = Some(Instant::now());
            relay.started |= outcome.ok;
            relay.last = Some(outcome);
        });
    match spawned {
        Ok(_) => true,
        Err(e) => {
            log::error!("Tongue: could not start the relay thread: {e}");
            relay().busy = false;
            false
        }
    }
}

pub fn shutdown() {
    let (script, port) = {
        let mut relay = relay();
        relay.wanted = false;
        let Some(config) = relay.config.as_ref() else {
            return;
        };
        if !relay.started && !relay.busy {
            return;
        }
        (config.script.clone(), config.port)
    };
    log::info!("Tongue: stopping headset camera relay");
    let outcome = execute(&script, port, "stop", STOP_TIMEOUT);
    if !outcome.ok {
        log::warn!(
            "Tongue: could not stop the headset relay: {}",
            outcome.output.trim()
        );
    }
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            let _ = sender.send(bytes);
        });
    }
    receiver
}

fn execute(script: &Path, port: u16, action: &str, timeout: Duration) -> Outcome {
    let mut command = Command::new("bash");
    command
        .arg(script)
        .arg(action)
        .env("PORT", port.to_string())
        .env("MODE", "mouth")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(directory) = script.parent() {
        command.current_dir(directory);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return Outcome {
                ok: false,
                output: format!("Could not run {}: {e}", script.display()),
            }
        }
    };
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                timed_out = true;
                unsafe {
                    libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
                }
                let _ = child.kill();
                break child.wait().ok();
            }
            Err(_) => break None,
        }
    };

    let collect = |receiver: mpsc::Receiver<Vec<u8>>| {
        receiver
            .recv_timeout(PIPE_GRACE)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    };
    let mut text = collect(stdout);
    let stderr = collect(stderr);
    if !stderr.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&stderr);
    }
    if timed_out {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!(
            "{} {action} did not finish within {}s and was stopped (is adb hanging?)",
            script.display(),
            timeout.as_secs()
        ));
    }
    Outcome {
        ok: !timed_out && status.is_some_and(|status| status.success()),
        output: text,
    }
}
