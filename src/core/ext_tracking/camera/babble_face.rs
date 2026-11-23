use std::path::Path;

use ort::{environment::Environment, session::Session, value::Tensor};

use super::super::{
    babble::ADDR_TO_UNIFIED,
    unified::{UnifiedExpressions, UnifiedTrackingData},
};

pub const OUTPUT_COUNT: usize = 45;
pub const INPUT_SIZE: usize = 224;

pub const OUTPUT_NAMES: [&str; OUTPUT_COUNT] = [
    "cheekPuffLeft",
    "cheekPuffRight",
    "cheekSuckLeft",
    "cheekSuckRight",
    "jawOpen",
    "jawForward",
    "jawLeft",
    "jawRight",
    "noseSneerLeft",
    "noseSneerRight",
    "mouthFunnel",
    "mouthPucker",
    "mouthLeft",
    "mouthRight",
    "mouthRollUpper",
    "mouthRollLower",
    "mouthShrugUpper",
    "mouthShrugLower",
    "mouthClose",
    "mouthSmileLeft",
    "mouthSmileRight",
    "mouthFrownLeft",
    "mouthFrownRight",
    "mouthDimpleLeft",
    "mouthDimpleRight",
    "mouthUpperUpLeft",
    "mouthUpperUpRight",
    "mouthLowerDownLeft",
    "mouthLowerDownRight",
    "mouthPressLeft",
    "mouthPressRight",
    "mouthStretchLeft",
    "mouthStretchRight",
    "tongueOut",
    "tongueUp",
    "tongueDown",
    "tongueLeft",
    "tongueRight",
    "tongueRoll",
    "tongueBendDown",
    "tongueCurlUp",
    "tongueSquish",
    "tongueFlat",
    "tongueTwistLeft",
    "tongueTwistRight",
];

pub struct Target {
    pub expression: UnifiedExpressions,
    pub mapper: Option<fn(f32) -> f32>,
}

pub struct MouthMapping {
    targets: Vec<Vec<Target>>,
    owned: Vec<usize>,
}

impl MouthMapping {
    pub fn new() -> Self {
        let targets: Vec<Vec<Target>> = OUTPUT_NAMES
            .iter()
            .map(|name| {
                ADDR_TO_UNIFIED
                    .get(format!("/{name}").as_str())
                    .map(|mappings| {
                        mappings
                            .iter()
                            .map(|m| Target {
                                expression: m.expression,
                                mapper: m.value_mapper,
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect();
        let mut owned: Vec<usize> = targets
            .iter()
            .flatten()
            .map(|t| t.expression as usize)
            .collect();
        owned.sort_unstable();
        owned.dedup();
        Self { targets, owned }
    }

    pub fn unmapped(&self) -> Vec<&'static str> {
        OUTPUT_NAMES
            .iter()
            .zip(&self.targets)
            .filter(|(_, targets)| targets.is_empty())
            .map(|(name, _)| *name)
            .collect()
    }

    pub fn apply(&self, values: &[f32], data: &mut UnifiedTrackingData) {
        for &index in &self.owned {
            data.shapes[index] = 0.0;
        }
        for (value, targets) in values.iter().zip(&self.targets) {
            for target in targets {
                let value = target.mapper.map_or(*value, |mapper| mapper(*value));
                let shape = &mut data.shapes[target.expression as usize];
                *shape = shape.max(value.clamp(0.0, 1.0));
            }
        }
    }
}

pub struct FaceModel {
    session: Session,
    input_name: String,
    pub size: usize,
    pub channels: usize,
}

impl FaceModel {
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
        let (size, channels) = match input.dtype() {
            ort::value::ValueType::Tensor { shape, .. } if shape.len() == 4 => {
                let dim = |i: usize, fallback: usize| {
                    usize::try_from(shape[i])
                        .ok()
                        .filter(|d| *d > 0)
                        .unwrap_or(fallback)
                };
                (dim(2, INPUT_SIZE), dim(1, 1))
            }
            ort::value::ValueType::Tensor { .. } => anyhow::bail!(
                "{}: expected a [1,1,224,224] image input, is this the Babble face model?",
                path.display()
            ),
            other => anyhow::bail!("{}: unsupported input type {other:?}", path.display()),
        };
        if !(16..=1024).contains(&size) || !matches!(channels, 1 | 3) {
            anyhow::bail!(
                "{}: unsupported input {channels}x{size}x{size}",
                path.display()
            );
        }
        let input_name = input.name().to_string();
        log::info!(
            "Babble camera: loaded {} (input {input_name} {channels}x{size}x{size})",
            path.display()
        );
        Ok(Self {
            session,
            input_name,
            size,
            channels,
        })
    }

    pub fn infer(&mut self, input: Vec<f32>) -> anyhow::Result<[f32; OUTPUT_COUNT]> {
        let tensor = Tensor::from_array(([1usize, self.channels, self.size, self.size], input))?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => tensor])?;
        let (_, values) = outputs[0].try_extract_tensor::<f32>()?;
        if values.len() < OUTPUT_COUNT {
            anyhow::bail!(
                "model returned {} values, expected {OUTPUT_COUNT} (is this the Babble face model?)",
                values.len()
            );
        }
        let mut out = [0f32; OUTPUT_COUNT];
        for (out, value) in out.iter_mut().zip(values) {
            *out = if value.is_finite() { *value } else { 0.0 };
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ext_tracking::unified::UnifiedExpressions as U;

    fn output(name: &str) -> usize {
        OUTPUT_NAMES.iter().position(|n| *n == name).unwrap()
    }

    #[test]
    fn every_babble_output_has_a_unified_target() {
        assert!(MouthMapping::new().unmapped().is_empty());
    }

    #[test]
    fn mouth_sideways_drives_lip_direction_not_press() {
        let mapping = MouthMapping::new();
        let mut values = [0f32; OUTPUT_COUNT];
        values[output("mouthLeft")] = 0.7;
        values[output("mouthPressRight")] = 0.3;
        let mut data = UnifiedTrackingData::default();
        data.setu(U::MouthPressLeft, 0.9);
        data.setu(U::EyeClosedLeft, 0.5);
        mapping.apply(&values, &mut data);
        assert_eq!(data.getu(U::MouthUpperLeft), 0.7);
        assert_eq!(data.getu(U::MouthLowerLeft), 0.7);
        assert_eq!(data.getu(U::MouthPressLeft), 0.0);
        assert_eq!(data.getu(U::MouthPressRight), 0.3);
        assert_eq!(data.getu(U::EyeClosedLeft), 0.5);
    }

    #[test]
    fn shared_targets_take_the_strongest_output() {
        let mapping = MouthMapping::new();
        let mut values = [0f32; OUTPUT_COUNT];
        values[output("mouthFrownLeft")] = 0.6;
        values[output("mouthStretchLeft")] = 0.2;
        values[output("jawOpen")] = 1.5;
        let mut data = UnifiedTrackingData::default();
        mapping.apply(&values, &mut data);
        assert_eq!(data.getu(U::MouthStretchLeft), 0.6);
        assert_eq!(data.getu(U::MouthFrownLeft), 0.6);
        assert_eq!(data.getu(U::JawOpen), 1.0);
    }
}
