#![allow(dead_code)]

use crate::core::{live::LIVE, AvatarOsc};

use clap::Parser;
use env_logger::Env;
use indicatif::MultiProgress;
use indicatif_log_bridge::LogWrapper;
use std::time::{Duration, Instant};

mod core;
#[cfg(feature = "gui")]
mod gui;

struct CaptureLog<L> {
    inner: L,
}

impl<L: log::Log> log::Log for CaptureLog<L> {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        if LIVE.enabled() && self.inner.enabled(record.metadata()) {
            LIVE.push_log(record.level(), &record.args().to_string());
        }
        self.inner.log(record);
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

#[cfg(feature = "gui")]
fn exit_now(code: i32) -> ! {
    use std::io::Write;
    #[cfg(feature = "qpro-tongue")]
    crate::core::ext_tracking::qpro_tongue::relay::shutdown();
    log::logger().flush();
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe { libc::_exit(code) }
}

fn main() {
    let log = env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .filter_module("mdns_sd", log::LevelFilter::Warn)
        .filter_module("winit", log::LevelFilter::Warn)
        .filter_module("eframe", log::LevelFilter::Warn)
        .filter_module("egui_glow", log::LevelFilter::Warn)
        .filter_module("sctk", log::LevelFilter::Warn)
        .format_target(false)
        .format_module_path(false)
        .build();
    let multi = MultiProgress::new();
    LogWrapper::new(multi.clone(), CaptureLog { inner: log })
        .try_init()
        .unwrap();

    let args = Args::parse();

    #[cfg(feature = "qpro-tongue")]
    signals::install();

    #[cfg(feature = "gui")]
    if args.gui {
        colored::control::set_override(true);
        LIVE.enable();
        let osc = std::thread::Builder::new()
            .name("osc".into())
            .spawn(move || {
                struct ExitOnDrop;
                impl Drop for ExitOnDrop {
                    fn drop(&mut self) {
                        exit_now(1);
                    }
                }
                let _exit = ExitOnDrop;
                run_osc(args, multi);
            })
            .expect("spawn osc thread");
        match gui::run() {
            Ok(()) => exit_now(0),
            Err(e) => {
                log::error!("GUI: {e:#}. Continuing without a window.");
                let _ = osc.join();
                return;
            }
        }
    }

    run_osc(args, multi);
}

fn run_osc(args: Args, multi: MultiProgress) {
    const MIN_BACKOFF: Duration = Duration::from_secs(1);
    const MAX_BACKOFF: Duration = Duration::from_secs(30);
    const HEALTHY: Duration = Duration::from_secs(60);
    let mut backoff = MIN_BACKOFF;
    loop {
        let started = Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            AvatarOsc::new(args.clone(), multi.clone()).handle_messages();
        }));
        if started.elapsed() > HEALTHY {
            backoff = MIN_BACKOFF;
        }
        match result {
            Ok(()) => log::warn!("OSC loop stopped, restarting in {}s", backoff.as_secs()),
            Err(panic) => {
                let reason = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".into());
                log::error!(
                    "OSC loop crashed: {reason}. Restarting in {}s",
                    backoff.as_secs()
                );
            }
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

#[derive(Default, Debug, Clone, clap::Subcommand)]
pub enum FaceSetup {
    #[default]
    #[clap(subcommand, hide = true)]
    /// Do not use face tracking
    Dummy,
    #[cfg(feature = "openxr")]
    /// Retrieve face data from OpenXR (WiVRn / Monado)
    Openxr(core::OpenXrArgs),

    #[cfg(feature = "alvr")]
    /// Retrieve face data from ALVR
    Alvr,

    #[cfg(feature = "babble")]
    /// Retrieve face data from Babble and Etvr
    Babble {
        /// The port to listen on for Babble and ETVR packets.
        #[arg(short, long, default_value = "9400")]
        listen: u16,

        #[cfg(feature = "cameras")]
        #[command(flatten)]
        cameras: core::ext_tracking::camera::CameraArgs,
    },

    #[command(
        about = "Retrieve ARKit face data from a phone app (iFacialMocap, VTube Studio iOS, MeowFace)"
    )]
    Phone(core::ext_tracking::PhoneArgs),

    #[command(
        about = "Retrieve face and eye data from SteamVR Steam Link (Steam Frame, Quest Pro)"
    )]
    Steamlink {
        #[arg(
            short,
            long,
            default_value = "9015",
            value_parser = clap::value_parser!(u16).range(1..),
            help = "The port to listen on; set Steam Link's OSC Output Port to the same value"
        )]
        listen: u16,
    },
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum FtMode {
    /// Native when the avatar reports Use All Tracking Keys, legacy parameters otherwise
    #[default]
    Auto,
    /// Only send avatar parameters (VRCFT-style)
    Legacy,
    /// Only send VRChat native face tracking (/tracking/face/v1)
    Native,
    /// Send both native face tracking and avatar parameters
    Both,
}

#[derive(Default, Clone, clap::Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Provider to use for face data
    #[command(subcommand)]
    face: FaceSetup,

    /// Expose and lisen on 0.0.0.0, instead of localhost
    #[arg(long, default_value_t = false)]
    expose: bool,

    /// OSC port for VRC
    #[arg(long, default_value = "9000")]
    vrc_port: u16,

    /// OSC listen port
    #[arg(long, default_value = "9002")]
    osc_port: u16,

    /// The OSC-JSON avatar file to use. See ~/.config/visage/avatar.json
    #[arg(long)]
    avatar: Option<String>,

    /// How face tracking is sent to VRC
    #[arg(long, value_enum, default_value_t = FtMode::Auto)]
    ft_mode: FtMode,

    #[cfg(feature = "gui")]
    #[arg(
        long,
        help = "Open a window with live face data and tongue tuning controls"
    )]
    gui: bool,
}

#[cfg(all(test, feature = "openxr", feature = "babble"))]
mod tests {
    use super::*;

    #[test]
    fn combined_openxr_babble_accepts_default_and_custom_ports() {
        for (tail, port) in [
            (vec!["--babble", "--independent-eyes"], 9400),
            (vec!["--babble", "9500", "--independent-eyes"], 9500),
        ] {
            let mut command = vec!["visage", "openxr"];
            command.extend(tail);
            let args = Args::try_parse_from(command).unwrap();
            let FaceSetup::Openxr(args) = args.face else {
                panic!("expected OpenXR")
            };
            assert_eq!(args.babble, Some(port));
            assert!(args.independent_eyes);
        }
        assert!(Args::try_parse_from(["visage", "openxr", "--babble", "0"]).is_err());
    }

    #[cfg(feature = "qpro-tongue")]
    #[test]
    fn babble_and_camera_tongue_cannot_write_the_same_mouth() {
        assert!(Args::try_parse_from([
            "visage",
            "openxr",
            "--babble",
            "--tongue-model",
            "gate.onnx"
        ])
        .is_err());
    }
}

#[cfg(feature = "qpro-tongue")]
mod signals {
    use std::sync::atomic::{AtomicI32, Ordering};

    static WRITE_FD: AtomicI32 = AtomicI32::new(-1);

    extern "C" fn on_signal(signal: libc::c_int) {
        let fd = WRITE_FD.load(Ordering::Relaxed);
        if fd >= 0 {
            let byte = signal as u8;
            unsafe {
                libc::write(fd, &byte as *const u8 as *const libc::c_void, 1);
            }
        }
    }

    pub fn install() {
        let mut fds = [0 as libc::c_int; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return;
        }
        let [read_fd, write_fd] = fds;
        WRITE_FD.store(write_fd, Ordering::Relaxed);
        let spawned = std::thread::Builder::new()
            .name("signals".into())
            .spawn(move || {
                let mut byte = 0u8;
                let read =
                    unsafe { libc::read(read_fd, &mut byte as *mut u8 as *mut libc::c_void, 1) };
                if read != 1 {
                    return;
                }
                log::info!("Received signal {byte}, shutting down");
                crate::core::ext_tracking::qpro_tongue::relay::shutdown();
                log::logger().flush();
                unsafe { libc::_exit(128 + byte as libc::c_int) };
            });
        if spawned.is_err() {
            return;
        }
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_signal as *const () as libc::sighandler_t;
                action.sa_flags = libc::SA_RESTART;
                libc::sigemptyset(&mut action.sa_mask);
                libc::sigaction(signal, &action, std::ptr::null_mut());
            }
        }
    }
}
