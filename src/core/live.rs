use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use glam::Vec3;
use once_cell::sync::Lazy;

use crate::FtMode;

use super::ext_tracking::unified::NUM_SHAPES;

const LOG_LINES: usize = 1000;
const LOG_LINE_CHARS: usize = 600;

#[derive(Clone)]
pub struct Snapshot {
    pub shapes: [f32; NUM_SHAPES],
    pub eyes: [Option<Vec3>; 2],
    pub status: Vec<Arc<str>>,
    pub ft_mode: FtMode,
    pub sending_native: bool,
    pub sending_legacy: bool,
    pub avatar_native_ft: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            shapes: [0.0; NUM_SHAPES],
            eyes: [None; 2],
            status: Vec::new(),
            ft_mode: FtMode::default(),
            sending_native: false,
            sending_legacy: false,
            avatar_native_ft: false,
        }
    }
}

pub struct Controls {
    pub ft_mode: Option<FtMode>,
    pub overrides: [Option<f32>; NUM_SHAPES],
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            ft_mode: None,
            overrides: [None; NUM_SHAPES],
        }
    }
}

#[derive(Clone)]
pub struct LogLine {
    pub level: log::Level,
    pub text: Arc<str>,
}

#[derive(Default)]
pub struct Live {
    enabled: AtomicBool,
    pub snapshot: Mutex<Snapshot>,
    pub controls: Mutex<Controls>,
    pub log: Mutex<VecDeque<LogLine>>,
}

impl Live {
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Relaxed);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn push_log(&self, level: log::Level, text: &str) {
        if !self.enabled() {
            return;
        }
        let text: Arc<str> = plain_text(text).into();
        let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        while log.len() >= LOG_LINES {
            log.pop_front();
        }
        log.push_back(LogLine { level, text });
    }
}

pub static LIVE: Lazy<Live> = Lazy::new(Live::default);

fn plain_text(text: &str) -> String {
    let mut plain = String::with_capacity(text.len().min(LOG_LINE_CHARS + 1));
    let mut chars = text.chars().peekable();
    let mut count = 0;
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if count == LOG_LINE_CHARS {
            plain.push('…');
            break;
        }
        plain.push(c);
        count += 1;
    }
    plain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_strips_ansi_and_caps_length() {
        assert_eq!(plain_text("\x1b[1;32mok\x1b[0m done"), "ok done");
        let long = "x".repeat(LOG_LINE_CHARS * 3);
        assert_eq!(plain_text(&long).chars().count(), LOG_LINE_CHARS + 1);
    }

    #[test]
    fn log_ring_is_bounded() {
        let live = Live::default();
        live.enable();
        for i in 0..LOG_LINES * 3 {
            live.push_log(log::Level::Info, &format!("line {i}"));
        }
        let log = live.log.lock().unwrap();
        assert_eq!(log.len(), LOG_LINES);
        assert_eq!(
            &*log.back().unwrap().text,
            format!("line {}", LOG_LINES * 3 - 1)
        );
    }
}
