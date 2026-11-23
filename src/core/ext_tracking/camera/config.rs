use std::path::PathBuf;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::core::folders::config_file;

use super::{
    babble_face, calib::Ranges, etvr_eyes::EyeSettings, onefilter::FilterParams, process::View,
    CameraId,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MouthConfig {
    pub source: Option<String>,
    pub model: Option<PathBuf>,
    pub view: View,
    pub filter: FilterParams,
    pub calibration: Ranges,
}

impl Default for MouthConfig {
    fn default() -> Self {
        Self {
            source: None,
            model: None,
            view: View::default(),
            filter: FilterParams::default(),
            calibration: Ranges::identity(babble_face::OUTPUT_COUNT),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EyeCameraConfig {
    pub source: Option<String>,
    pub view: View,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EyesConfig {
    pub left: EyeCameraConfig,
    pub right: EyeCameraConfig,
    pub model: Option<PathBuf>,
    pub filter: FilterParams,
    pub settings: EyeSettings,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraConfig {
    pub mouth: MouthConfig,
    pub eyes: EyesConfig,
}

fn clean_source(source: Option<String>) -> Option<String> {
    source
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn clean_path(path: Option<PathBuf>) -> Option<PathBuf> {
    path.filter(|p| !p.as_os_str().is_empty())
}

impl CameraConfig {
    pub fn path() -> PathBuf {
        static PATH: Lazy<PathBuf> = Lazy::new(|| PathBuf::from(config_file("cameras.json", None)));
        PATH.clone()
    }

    pub fn load() -> anyhow::Result<Option<Self>> {
        let path = Self::path();
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(
                serde_json::from_slice::<Self>(&bytes)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?
                    .sanitized(),
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(anyhow::anyhow!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self) -> anyhow::Result<PathBuf> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&temporary, &path)?;
        Ok(path)
    }

    pub fn sanitized(mut self) -> Self {
        self.mouth.source = clean_source(self.mouth.source);
        self.mouth.model = clean_path(self.mouth.model);
        self.mouth.view = self.mouth.view.sanitized();
        self.mouth.filter = self.mouth.filter.sanitized();
        self.mouth.calibration = self.mouth.calibration.sanitized(babble_face::OUTPUT_COUNT);
        for eye in [&mut self.eyes.left, &mut self.eyes.right] {
            eye.source = clean_source(eye.source.take());
            eye.view = std::mem::take(&mut eye.view).sanitized();
        }
        self.eyes.model = clean_path(self.eyes.model);
        self.eyes.filter = self.eyes.filter.sanitized();
        self.eyes.settings = self.eyes.settings.sanitized();
        self
    }

    pub fn source(&self, id: CameraId) -> Option<&str> {
        match id {
            CameraId::Mouth => self.mouth.source.as_deref(),
            CameraId::Left => self.eyes.left.source.as_deref(),
            CameraId::Right => self.eyes.right.source.as_deref(),
        }
    }

    pub fn set_source(&mut self, id: CameraId, source: Option<String>) {
        let source = clean_source(source);
        match id {
            CameraId::Mouth => self.mouth.source = source,
            CameraId::Left => self.eyes.left.source = source,
            CameraId::Right => self.eyes.right.source = source,
        }
    }

    pub fn view(&self, id: CameraId) -> &View {
        match id {
            CameraId::Mouth => &self.mouth.view,
            CameraId::Left => &self.eyes.left.view,
            CameraId::Right => &self.eyes.right.view,
        }
    }

    pub fn view_mut(&mut self, id: CameraId) -> &mut View {
        match id {
            CameraId::Mouth => &mut self.mouth.view,
            CameraId::Left => &mut self.eyes.left.view,
            CameraId::Right => &mut self.eyes.right.view,
        }
    }

    pub fn mouth_enabled(&self) -> bool {
        self.mouth.source.is_some()
    }

    pub fn eyes_enabled(&self) -> bool {
        self.eyes.left.source.is_some() || self.eyes.right.source.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_and_fills_missing_fields() {
        let mut config = CameraConfig::default();
        config.mouth.source = Some("/dev/ttyUSB0".into());
        config.mouth.view.rotation = 30.0;
        config.eyes.settings.gaze.y.offset = 0.1;
        let text = serde_json::to_string(&config).unwrap();
        let back: CameraConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(back, config);

        let partial: CameraConfig = serde_json::from_str(
            r#"{"mouth": {"source": "  ", "view": {"rotation": 540, "roi": {"x": 2}}, "calibration": {"lower": [0.1]}}}"#,
        )
        .unwrap();
        let partial = partial.sanitized();
        assert_eq!(partial.mouth.source, None);
        assert_eq!(partial.mouth.view.rotation, -180.0);
        assert!(partial.mouth.view.roi.x <= 0.98);
        assert_eq!(
            partial.mouth.calibration.lower.len(),
            babble_face::OUTPUT_COUNT
        );
        assert_eq!(partial.mouth.calibration.lower[0], 0.1);
        assert_eq!(partial.eyes.settings.open_lid, 0.75);
        assert!(!partial.mouth_enabled() && !partial.eyes_enabled());
    }
}
