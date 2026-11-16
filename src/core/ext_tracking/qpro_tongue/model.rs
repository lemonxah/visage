use std::path::Path;

use ort::{environment::Environment, session::Session, value::Tensor};

use super::stream::{Frame, PANEL};

const DEFAULT_CAMERA_WEIGHT: f32 = 0.5;
const DEFAULT_THRESHOLD: f32 = 0.44;

#[derive(Clone, Copy, Debug)]
pub struct VisibilityGate {
    pub camera_weight: f32,
    pub threshold: f32,
}

pub struct TongueModel {
    session: Session,
    input_name: String,
    resampler: AreaResampler,
    pub targets: Vec<String>,
    pub gate: VisibilityGate,
}

impl TongueModel {
    pub fn load(environment: &Environment, path: &Path, threads: usize) -> anyhow::Result<Self> {
        let session = Session::builder(environment)
            .and_then(|b| b.with_intra_threads(threads).map_err(Into::into))
            .and_then(|b| b.with_intra_op_spinning(false).map_err(Into::into))
            .and_then(|mut b| b.commit_from_file(path))
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;

        let metadata = session.metadata()?;
        let preprocessing = metadata
            .custom("inputPreprocessing")
            .unwrap_or_else(|| "raw-v1".into());
        if preprocessing != "raw-v1" {
            anyhow::bail!(
                "{}: unsupported input preprocessing {preprocessing}",
                path.display()
            );
        }
        let targets: Vec<String> = serde_json::from_str(
            &metadata
                .custom("targetNames")
                .ok_or_else(|| anyhow::anyhow!("{}: no targetNames metadata", path.display()))?,
        )?;
        let image_size: usize = metadata
            .custom("imageSize")
            .ok_or_else(|| anyhow::anyhow!("{}: no imageSize metadata", path.display()))?
            .parse()?;
        if image_size == 0 || image_size > PANEL {
            anyhow::bail!("{}: unsupported image size {image_size}", path.display());
        }
        let gate: serde_json::Value = metadata
            .custom("visibilityGate")
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let gate = VisibilityGate {
            camera_weight: gate["cameraWeight"]
                .as_f64()
                .map_or(DEFAULT_CAMERA_WEIGHT, |v| v as f32),
            threshold: gate["threshold"]
                .as_f64()
                .map_or(DEFAULT_THRESHOLD, |v| v as f32),
        };
        drop(metadata);
        log::info!(
            "Tongue: loaded {} ({}px, {})",
            path.display(),
            image_size,
            preprocessing
        );

        let input_name = session
            .inputs()
            .first()
            .ok_or_else(|| anyhow::anyhow!("{}: model has no inputs", path.display()))?
            .name()
            .to_string();

        Ok(Self {
            session,
            input_name,
            resampler: AreaResampler::new(PANEL, image_size),
            targets,
            gate,
        })
    }

    pub fn infer(&mut self, frame: &Frame) -> anyhow::Result<Vec<f32>> {
        let size = self.resampler.target;
        let mut cameras = vec![0f32; 2 * size * size];
        let (left, right) = cameras.split_at_mut(size * size);
        self.resampler.resample(&frame.left, left);
        self.resampler.resample(&frame.right, right);

        let input = Tensor::from_array(([1usize, 2, size, size], cameras))?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => input])?;
        let (_, values) = outputs[0].try_extract_tensor::<f32>()?;
        if values.len() != self.targets.len() {
            anyhow::bail!(
                "model returned {} values for {} targets",
                values.len(),
                self.targets.len()
            );
        }
        Ok(values.to_vec())
    }
}

struct Tap {
    start: usize,
    weights: Vec<f32>,
}

pub struct AreaResampler {
    source: usize,
    target: usize,
    taps: Vec<Tap>,
    rows: Vec<f32>,
}

impl AreaResampler {
    pub fn new(source: usize, target: usize) -> Self {
        let scale = source as f64 / target as f64;
        let taps = (0..target)
            .map(|index| {
                let begin = index as f64 * scale;
                let end = ((index + 1) as f64 * scale).min(source as f64);
                let start = begin.floor() as usize;
                let stop = (end.ceil() as usize).clamp(start + 1, source);
                let weights = (start..stop)
                    .map(|pixel| {
                        let overlap =
                            (end.min(pixel as f64 + 1.0) - begin.max(pixel as f64)).max(0.0);
                        (overlap / (end - begin)) as f32
                    })
                    .collect();
                Tap { start, weights }
            })
            .collect();

        Self {
            source,
            target,
            taps,
            rows: vec![0.0; source * target],
        }
    }

    pub fn resample(&mut self, image: &[u8], output: &mut [f32]) {
        assert_eq!(image.len(), self.source * self.source);
        assert_eq!(output.len(), self.target * self.target);

        for (source_row, row) in image
            .chunks_exact(self.source)
            .zip(self.rows.chunks_exact_mut(self.target))
        {
            for (value, tap) in row.iter_mut().zip(&self.taps) {
                *value = tap
                    .weights
                    .iter()
                    .zip(&source_row[tap.start..])
                    .map(|(weight, pixel)| weight * *pixel as f32)
                    .sum();
            }
        }

        for (output_row, tap) in output.chunks_exact_mut(self.target).zip(&self.taps) {
            output_row.fill(0.0);
            for (offset, weight) in tap.weights.iter().enumerate() {
                let row = &self.rows[(tap.start + offset) * self.target..][..self.target];
                for (value, source) in output_row.iter_mut().zip(row) {
                    *value += weight * source;
                }
            }
            for value in output_row.iter_mut() {
                *value = value.round().clamp(0.0, 255.0) / 255.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_ratio_is_a_box_average() {
        let mut image = vec![0u8; 16];
        image[..4].copy_from_slice(&[10, 30, 100, 100]);
        image[4..8].copy_from_slice(&[50, 70, 100, 100]);
        let mut output = vec![0f32; 4];
        AreaResampler::new(4, 2).resample(&image, &mut output);
        assert!((output[0] - 40.0 / 255.0).abs() < 1e-6);
        assert!((output[1] - 100.0 / 255.0).abs() < 1e-6);
        assert_eq!(output[2], 0.0);
    }

    #[test]
    fn fractional_ratio_weights_partial_pixels() {
        let image: Vec<u8> = [0u8, 90, 180].repeat(3);
        let mut output = vec![0f32; 4];
        AreaResampler::new(3, 2).resample(&image, &mut output);
        assert!((output[0] - 30.0 / 255.0).abs() < 1e-6);
        assert!((output[1] - 150.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn matches_reference_when_provided() {
        let Ok(directory) = std::env::var("QPRO_TONGUE_REFERENCE") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        let strip = std::fs::read(directory.join("strip.bin")).unwrap();
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("expected.json")).unwrap())
                .unwrap();
        let frame = Frame {
            left: strip
                .chunks_exact(2 * PANEL)
                .flat_map(|row| row[..PANEL].to_vec())
                .collect(),
            right: strip
                .chunks_exact(2 * PANEL)
                .flat_map(|row| row[PANEL..].to_vec())
                .collect(),
        };

        let environment = ort::init().with_name("test").build().unwrap();
        for case in expected.as_array().unwrap() {
            let path = std::path::PathBuf::from(case["model"].as_str().unwrap());
            let mut model = TongueModel::load(&environment, &path, 2).unwrap();
            let values = model.infer(&frame).unwrap();
            let started = std::time::Instant::now();
            for _ in 0..20 {
                model.infer(&frame).unwrap();
            }
            println!(
                "{}: {:.1} ms per frame",
                path.display(),
                started.elapsed().as_secs_f32() * 50.0
            );
            let reference = case["values"].as_array().unwrap();
            assert_eq!(values.len(), reference.len());
            let mut max_error = 0f32;
            for (index, (value, reference)) in values.iter().zip(reference).enumerate() {
                let error = (value - reference.as_f64().unwrap() as f32).abs();
                assert!(
                    error < 2e-3,
                    "{}: output {index}, error {error}",
                    path.display()
                );
                max_error = max_error.max(error);
            }
            println!("{}: max reference error {max_error:.7}", path.display());
        }
    }

    #[test]
    fn flat_image_keeps_its_level() {
        let image = vec![200u8; PANEL * PANEL];
        for target in [192usize, 224] {
            let mut output = vec![0f32; target * target];
            AreaResampler::new(PANEL, target).resample(&image, &mut output);
            assert!(output.iter().all(|v| (v - 200.0 / 255.0).abs() < 1e-6));
        }
    }
}
