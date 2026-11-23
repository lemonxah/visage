use std::path::Path;

use glam::vec3;
use ort::{environment::Environment, session::Session, value::Tensor};
use serde::{Deserialize, Serialize};

use super::{
    super::unified::{UnifiedExpressions, UnifiedTrackingData},
    calib::{Axis, Ranges},
};

pub const OUTPUT_COUNT: usize = 12;
pub const INPUT_SIZE: usize = 192;
pub const INPUT_CHANNELS: usize = 6;
pub const EXPRESSION_COUNT: usize = 4;

pub const OUTPUT_NAMES: [&str; OUTPUT_COUNT] = [
    "eyebrow_l",
    "eyelid_l",
    "squeeze_l",
    "eyebrow_r",
    "eyelid_r",
    "squeeze_r",
    "gaze_x",
    "gaze_y",
    "gaze_x_l",
    "gaze_y_l",
    "gaze_x_r",
    "gaze_y_r",
];

const BROW_L: usize = 0;
const LID_L: usize = 1;
const SQUEEZE_L: usize = 2;
const BROW_R: usize = 3;
const LID_R: usize = 4;
const SQUEEZE_R: usize = 5;
const GAZE_X: usize = 6;
const GAZE_Y: usize = 7;
const GAZE_X_L: usize = 8;
const GAZE_Y_L: usize = 9;
const GAZE_X_R: usize = 10;
const GAZE_Y_R: usize = 11;

pub const EXPRESSION_NAMES: [&str; EXPRESSION_COUNT] =
    ["brow left", "brow right", "squeeze left", "squeeze right"];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GazeLimits {
    pub horizontal: f32,
    pub up: f32,
    pub down: f32,
}

impl Default for GazeLimits {
    fn default() -> Self {
        Self {
            horizontal: 40.0,
            up: 15.0,
            down: 40.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GazeCalibration {
    pub left_x: Axis,
    pub right_x: Axis,
    pub y: Axis,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EyeSettings {
    pub open_lid: f32,
    pub wide_lid: f32,
    pub limits: GazeLimits,
    pub gaze: GazeCalibration,
    pub expressions: Ranges,
}

impl Default for EyeSettings {
    fn default() -> Self {
        Self {
            open_lid: 0.75,
            wide_lid: 0.95,
            limits: GazeLimits::default(),
            gaze: GazeCalibration::default(),
            expressions: Ranges::identity(EXPRESSION_COUNT),
        }
    }
}

impl EyeSettings {
    pub fn sanitized(mut self) -> Self {
        let defaults = Self::default();
        let unit = |v: f32, fallback: f32| {
            if v.is_finite() {
                v.clamp(0.05, 1.0)
            } else {
                fallback
            }
        };
        self.open_lid = unit(self.open_lid, defaults.open_lid);
        self.wide_lid = unit(self.wide_lid, defaults.wide_lid).max(self.open_lid + 0.01);
        let angle = |v: f32, fallback: f32| {
            if v.is_finite() {
                v.clamp(1.0, 90.0)
            } else {
                fallback
            }
        };
        self.limits = GazeLimits {
            horizontal: angle(self.limits.horizontal, defaults.limits.horizontal),
            up: angle(self.limits.up, defaults.limits.up),
            down: angle(self.limits.down, defaults.limits.down),
        };
        self.gaze = GazeCalibration {
            left_x: self.gaze.left_x.sanitized(),
            right_x: self.gaze.right_x.sanitized(),
            y: self.gaze.y.sanitized(),
        };
        self.expressions = self.expressions.sanitized(EXPRESSION_COUNT);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EyeState {
    pub closed: [f32; 2],
    pub wide: [f32; 2],
    pub squint: [f32; 2],
    pub brow: [f32; 2],
    pub gaze_x: [f32; 2],
    pub gaze_y: f32,
}

pub fn lid_shapes(lid: f32, open: f32, wide: f32) -> (f32, f32) {
    if !lid.is_finite() {
        return (0.0, 0.0);
    }
    let closed = 1.0 - (lid / open).clamp(0.0, 1.0);
    let widened = ((lid - open) / (wide - open).max(1e-3)).clamp(0.0, 1.0);
    (closed, widened)
}

pub fn expressions(values: &[f32; OUTPUT_COUNT]) -> [f32; EXPRESSION_COUNT] {
    [
        values[BROW_L],
        values[BROW_R],
        values[SQUEEZE_L],
        values[SQUEEZE_R],
    ]
}

pub fn gaze_raw(values: &[f32; OUTPUT_COUNT]) -> [f32; 3] {
    [values[GAZE_X_L], values[GAZE_X_R], values[GAZE_Y]]
}

pub fn mirror_missing(values: &mut [f32; OUTPUT_COUNT], left: bool, right: bool) {
    let pairs = [
        (BROW_L, BROW_R),
        (LID_L, LID_R),
        (SQUEEZE_L, SQUEEZE_R),
        (GAZE_X_L, GAZE_X_R),
        (GAZE_Y_L, GAZE_Y_R),
    ];
    if !left && right {
        for (l, r) in pairs {
            values[l] = values[r];
        }
        values[GAZE_X] = values[GAZE_X_R];
        values[GAZE_Y] = values[GAZE_Y_R];
    } else if left && !right {
        for (l, r) in pairs {
            values[r] = values[l];
        }
        values[GAZE_X] = values[GAZE_X_L];
        values[GAZE_Y] = values[GAZE_Y_L];
    }
}

pub fn compute(values: &[f32; OUTPUT_COUNT], settings: &EyeSettings) -> EyeState {
    let (closed_l, wide_l) = lid_shapes(values[LID_L], settings.open_lid, settings.wide_lid);
    let (closed_r, wide_r) = lid_shapes(values[LID_R], settings.open_lid, settings.wide_lid);
    let mut expression = expressions(values);
    settings.expressions.apply(&mut expression);
    EyeState {
        closed: [closed_l, closed_r],
        wide: [wide_l, wide_r],
        brow: [expression[0], expression[1]],
        squint: [expression[2], expression[3]],
        gaze_x: [
            settings.gaze.left_x.apply(values[GAZE_X_L]),
            settings.gaze.right_x.apply(values[GAZE_X_R]),
        ],
        gaze_y: settings.gaze.y.apply(values[GAZE_Y]),
    }
}

pub fn apply(state: &EyeState, limits: &GazeLimits, data: &mut UnifiedTrackingData) {
    use UnifiedExpressions as U;
    data.setu(U::EyeClosedLeft, state.closed[0]);
    data.setu(U::EyeClosedRight, state.closed[1]);
    data.setu(U::EyeWideLeft, state.wide[0]);
    data.setu(U::EyeWideRight, state.wide[1]);
    data.setu(U::EyeSquintLeft, state.squint[0]);
    data.setu(U::EyeSquintRight, state.squint[1]);
    data.setu(U::BrowInnerUpLeft, state.brow[0]);
    data.setu(U::BrowOuterUpLeft, state.brow[0]);
    data.setu(U::BrowInnerUpRight, state.brow[1]);
    data.setu(U::BrowOuterUpRight, state.brow[1]);
    data.setu(U::EyeLeftX, state.gaze_x[0]);
    data.setu(U::EyeRightX, state.gaze_x[1]);
    data.setu(U::EyeY, -state.gaze_y);

    let up = -state.gaze_y;
    let pitch = if up >= 0.0 {
        up * limits.up
    } else {
        up * limits.down
    }
    .to_radians();
    let yaw = |x: f32| (-x * limits.horizontal).to_radians();
    data.eyes = [
        Some(vec3(pitch, yaw(state.gaze_x[0]), 0.0)),
        Some(vec3(pitch, yaw(state.gaze_x[1]), 0.0)),
    ];
}

fn normalize(name: &str) -> String {
    let mut text: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    for (long, short) in [("left", "l"), ("right", "r")] {
        if let Some(stem) = text.strip_suffix(long) {
            text = format!("{stem}{short}");
        }
    }
    text
}

pub fn resolve(names: &[String]) -> Option<[usize; OUTPUT_COUNT]> {
    let normalized: Vec<String> = names.iter().map(|n| normalize(n)).collect();
    let mut order = [0usize; OUTPUT_COUNT];
    for (slot, wanted) in order.iter_mut().zip(OUTPUT_NAMES) {
        let wanted = normalize(wanted);
        *slot = normalized.iter().position(|n| *n == wanted)?;
    }
    Some(order)
}

pub fn candidate_lists(value: &str) -> Vec<Vec<String>> {
    let mut lists = Vec::new();
    match serde_json::from_str::<serde_json::Value>(value) {
        Ok(serde_json::Value::Array(items)) => {
            let names: Vec<String> = items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect();
            if names.len() == items.len() {
                lists.push(names);
            }
        }
        Ok(serde_json::Value::Object(map)) => {
            let mut by_index: Vec<(usize, String)> = map
                .iter()
                .filter_map(|(key, value)| {
                    value
                        .as_u64()
                        .map(|index| (index as usize, key.clone()))
                        .or_else(|| {
                            key.parse::<usize>()
                                .ok()
                                .zip(value.as_str().map(str::to_string))
                        })
                })
                .collect();
            if by_index.len() == map.len() && by_index.len() <= 256 {
                by_index.sort();
                let mut names = vec![String::new(); by_index.last().map_or(0, |(i, _)| i + 1)];
                for (index, name) in by_index {
                    names[index] = name;
                }
                lists.push(names);
            }
        }
        _ => {}
    }
    let split: Vec<String> = value
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .map(|s| s.trim_matches(|c: char| c == '"' || c == '\'' || c == '[' || c == ']'))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if split.len() >= OUTPUT_COUNT {
        lists.push(split);
    }
    lists
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Separate,
    Single,
}

pub fn choose_layout(
    outlet_names: &[String],
    metadata: &[(String, String)],
) -> (Layout, [usize; OUTPUT_COUNT], String) {
    let default: [usize; OUTPUT_COUNT] = std::array::from_fn(|i| i);
    if outlet_names.len() >= OUTPUT_COUNT {
        return match resolve(outlet_names) {
            Some(order) => (Layout::Separate, order, "output names".into()),
            None => (Layout::Separate, default, "default order".into()),
        };
    }
    let mut sorted: Vec<&(String, String)> = metadata.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, value) in sorted {
        for list in candidate_lists(value) {
            if let Some(order) = resolve(&list) {
                return (Layout::Single, order, format!("metadata \"{key}\""));
            }
        }
    }
    (Layout::Single, default, "default order".into())
}

pub struct EyeModel {
    session: Session,
    input_name: String,
    layout: Layout,
    order: [usize; OUTPUT_COUNT],
    pub size: usize,
}

impl EyeModel {
    pub fn load(environment: &Environment, path: &Path, threads: usize) -> anyhow::Result<Self> {
        let session = Session::builder(environment)
            .and_then(|b| b.with_intra_threads(threads).map_err(Into::into))
            .and_then(|b| b.with_intra_op_spinning(false).map_err(Into::into))
            .and_then(|mut b| b.commit_from_file(path))
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let input = session
            .inputs()
            .first()
            .ok_or_else(|| anyhow::anyhow!("{}: model has no inputs", path.display()))?;
        let size = match input.dtype() {
            ort::value::ValueType::Tensor { shape, .. } if shape.len() == 4 => {
                if shape[1] > 0 && shape[1] as usize != INPUT_CHANNELS {
                    anyhow::bail!(
                        "{}: input has {} channels; only the 6-channel EyeTrackVR NEXT stereo models are supported",
                        path.display(),
                        shape[1]
                    );
                }
                usize::try_from(shape[2])
                    .ok()
                    .filter(|d| *d > 0)
                    .unwrap_or(INPUT_SIZE)
            }
            other => anyhow::bail!("{}: unsupported input {other:?}", path.display()),
        };
        if !(16..=1024).contains(&size) {
            anyhow::bail!("{}: unsupported input size {size}", path.display());
        }
        let input_name = input.name().to_string();
        let outlet_names: Vec<String> = session
            .outputs()
            .iter()
            .map(|o| o.name().to_string())
            .collect();
        let metadata = session.metadata()?;
        let pairs: Vec<(String, String)> = metadata
            .custom_keys()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|key| metadata.custom(&key).map(|value| (key, value)))
            .collect();
        drop(metadata);
        let (layout, order, source) = choose_layout(&outlet_names, &pairs);
        log::info!(
            "ETVR camera: loaded {} (input {input_name} {INPUT_CHANNELS}x{size}x{size}, outputs from {source})",
            path.display()
        );
        Ok(Self {
            session,
            input_name,
            layout,
            order,
            size,
        })
    }

    pub fn infer(&mut self, input: Vec<f32>) -> anyhow::Result<[f32; OUTPUT_COUNT]> {
        let tensor = Tensor::from_array(([1usize, INPUT_CHANNELS, self.size, self.size], input))?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => tensor])?;
        let mut values = [0f32; OUTPUT_COUNT];
        match self.layout {
            Layout::Separate => {
                for (value, &index) in values.iter_mut().zip(&self.order) {
                    if index >= outputs.len() {
                        anyhow::bail!("model has fewer than {OUTPUT_COUNT} outputs");
                    }
                    let (_, data) = outputs[index].try_extract_tensor::<f32>()?;
                    *value = *data
                        .first()
                        .ok_or_else(|| anyhow::anyhow!("empty model output"))?;
                }
            }
            Layout::Single => {
                let (_, data) = outputs[0].try_extract_tensor::<f32>()?;
                for (value, &index) in values.iter_mut().zip(&self.order) {
                    *value = *data.get(index).ok_or_else(|| {
                        anyhow::anyhow!(
                            "model returned {} values, expected {OUTPUT_COUNT}",
                            data.len()
                        )
                    })?;
                }
            }
        }
        for value in values.iter_mut() {
            if !value.is_finite() {
                *value = 0.0;
            }
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ext_tracking::unified::UnifiedExpressions as U;

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn lid_convention_maps_open_closed_and_wide() {
        assert_eq!(lid_shapes(0.75, 0.75, 0.95), (0.0, 0.0));
        assert_eq!(lid_shapes(0.0, 0.75, 0.95), (1.0, 0.0));
        let (closed, wide) = lid_shapes(0.375, 0.75, 0.95);
        assert!((closed - 0.5).abs() < 1e-6 && wide == 0.0);
        let (closed, wide) = lid_shapes(0.85, 0.75, 0.95);
        assert!(closed == 0.0 && (wide - 0.5).abs() < 1e-5);
        assert_eq!(lid_shapes(1.0, 0.75, 0.95), (0.0, 1.0));
        assert_eq!(lid_shapes(f32::NAN, 0.75, 0.95), (0.0, 0.0));
    }

    #[test]
    fn output_names_resolve_in_any_order_and_case() {
        let mut names = strings(&OUTPUT_NAMES);
        names.reverse();
        names[0] = "Gaze_Y_Right".into();
        let order = resolve(&names).unwrap();
        assert_eq!(order[GAZE_Y_R], 0);
        assert_eq!(order[BROW_L], OUTPUT_COUNT - 1);
        assert!(resolve(&strings(&["eyebrow_l", "eyelid_l"])).is_none());
    }

    #[test]
    fn layout_prefers_outlets_then_metadata_then_default() {
        let mut shuffled = strings(&OUTPUT_NAMES);
        shuffled.swap(0, 11);
        let (layout, order, _) = choose_layout(&shuffled, &[]);
        assert_eq!(layout, Layout::Separate);
        assert_eq!(order[BROW_L], 11);
        assert_eq!(order[GAZE_Y_R], 0);

        let json = serde_json::to_string(&shuffled).unwrap();
        let metadata = vec![
            ("producer".to_string(), "pytorch".to_string()),
            ("output_names".to_string(), json),
        ];
        let (layout, order, source) = choose_layout(&strings(&["output"]), &metadata);
        assert_eq!(layout, Layout::Single);
        assert_eq!(order[BROW_L], 11);
        assert!(source.contains("output_names"));

        let object = r#"{"0":"gaze_y_r","1":"eyelid_l","2":"squeeze_l","3":"eyebrow_r","4":"eyelid_r","5":"squeeze_r","6":"gaze_x","7":"gaze_y","8":"gaze_x_l","9":"gaze_y_l","10":"gaze_x_r","11":"eyebrow_l"}"#;
        let (_, order, _) = choose_layout(&[], &[("names".into(), object.into())]);
        assert_eq!(order[BROW_L], 11);

        let csv = shuffled.join(", ");
        let (_, order, _) = choose_layout(&[], &[("labels".into(), csv)]);
        assert_eq!(order[BROW_L], 11);

        let (layout, order, source) = choose_layout(&strings(&["y"]), &[("x".into(), "1".into())]);
        assert_eq!(layout, Layout::Single);
        assert_eq!(order, std::array::from_fn(|i| i));
        assert_eq!(source, "default order");

        let unnamed: Vec<String> = (0..12).map(|i| format!("out{i}")).collect();
        let (layout, order, _) = choose_layout(&unnamed, &[]);
        assert_eq!(layout, Layout::Separate);
        assert_eq!(order, std::array::from_fn(|i| i));
    }

    #[test]
    fn missing_eye_copies_the_other() {
        let mut values = [0f32; OUTPUT_COUNT];
        values[LID_R] = 0.6;
        values[GAZE_X_R] = 0.4;
        values[GAZE_Y_R] = -0.2;
        values[LID_L] = 0.1;
        mirror_missing(&mut values, false, true);
        assert_eq!(values[LID_L], 0.6);
        assert_eq!(values[GAZE_X_L], 0.4);
        assert_eq!(values[GAZE_Y], -0.2);
    }

    #[test]
    fn eye_state_drives_unified_gaze_like_openxr() {
        let mut values = [0f32; OUTPUT_COUNT];
        values[LID_L] = 0.75;
        values[LID_R] = 0.0;
        values[BROW_L] = 0.4;
        values[SQUEEZE_R] = 0.3;
        values[GAZE_X_L] = 0.5;
        values[GAZE_X_R] = 0.5;
        values[GAZE_Y] = 1.0;
        let settings = EyeSettings::default();
        let state = compute(&values, &settings);
        let mut data = UnifiedTrackingData::default();
        apply(&state, &settings.limits, &mut data);
        assert_eq!(data.getu(U::EyeClosedLeft), 0.0);
        assert_eq!(data.getu(U::EyeClosedRight), 1.0);
        assert_eq!(data.getu(U::BrowInnerUpLeft), 0.4);
        assert_eq!(data.getu(U::EyeSquintRight), 0.3);
        assert_eq!(data.getu(U::EyeLeftX), 0.5);
        assert_eq!(data.getu(U::EyeY), -1.0);
        let left = data.eyes[0].unwrap();
        assert!((left.x.to_degrees() + 40.0).abs() < 1e-3);
        assert!((left.y.to_degrees() + 20.0).abs() < 1e-3);
        let payload = data.native_face_payload();
        assert!(
            payload[5] > 128 && payload[4] == 0,
            "left eye looks right (in)"
        );
        assert!(payload[7] == 255 && payload[6] == 0, "both eyes look down");

        values[GAZE_Y] = -1.0;
        let state = compute(&values, &settings);
        apply(&state, &settings.limits, &mut data);
        assert!((data.eyes[1].unwrap().x.to_degrees() - 15.0).abs() < 1e-3);
    }
}
