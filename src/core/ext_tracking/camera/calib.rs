use serde::{Deserialize, Serialize};

pub const MIN_SPAN: f32 = 0.02;
const MIN_GAZE_SPAN: f32 = 0.1;

pub fn remap(value: f32, lower: f32, upper: f32) -> f32 {
    let span = upper - lower;
    if !value.is_finite() {
        return 0.0;
    }
    if span.is_nan() || span < MIN_SPAN {
        return value.clamp(0.0, 1.0);
    }
    ((value - lower) / span).clamp(0.0, 1.0)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ranges {
    pub lower: Vec<f32>,
    pub upper: Vec<f32>,
}

impl Default for Ranges {
    fn default() -> Self {
        Self::identity(0)
    }
}

impl Ranges {
    pub fn identity(count: usize) -> Self {
        Self {
            lower: vec![0.0; count],
            upper: vec![1.0; count],
        }
    }

    pub fn sanitized(mut self, count: usize) -> Self {
        let fix = |values: &mut Vec<f32>, fallback: f32| {
            values.resize(count, fallback);
            for value in values.iter_mut() {
                *value = if value.is_finite() {
                    value.clamp(-1.0, 2.0)
                } else {
                    fallback
                };
            }
        };
        fix(&mut self.lower, 0.0);
        fix(&mut self.upper, 1.0);
        self
    }

    pub fn begin(&mut self, values: &[f32]) {
        self.lower = values
            .iter()
            .map(|v| if v.is_finite() { *v } else { 0.0 })
            .collect();
        self.upper = self.lower.clone();
    }

    pub fn observe(&mut self, values: &[f32]) {
        for ((lower, upper), value) in self.lower.iter_mut().zip(self.upper.iter_mut()).zip(values)
        {
            if value.is_finite() {
                *lower = lower.min(*value);
                *upper = upper.max(*value);
            }
        }
    }

    pub fn apply(&self, values: &mut [f32]) {
        for (index, value) in values.iter_mut().enumerate() {
            let lower = self.lower.get(index).copied().unwrap_or(0.0);
            let upper = self.upper.get(index).copied().unwrap_or(1.0);
            *value = remap(*value, lower, upper);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Axis {
    pub gain: f32,
    pub offset: f32,
}

impl Default for Axis {
    fn default() -> Self {
        Self {
            gain: 1.0,
            offset: 0.0,
        }
    }
}

impl Axis {
    pub fn apply(&self, value: f32) -> f32 {
        if !value.is_finite() {
            return 0.0;
        }
        ((value - self.offset) * self.gain).clamp(-1.0, 1.0)
    }

    pub fn sanitized(self) -> Self {
        Self {
            gain: if self.gain.is_finite() {
                self.gain.clamp(0.1, 5.0)
            } else {
                1.0
            },
            offset: if self.offset.is_finite() {
                self.offset.clamp(-1.0, 1.0)
            } else {
                0.0
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub min: f32,
    pub max: f32,
}

impl Extent {
    pub fn new(value: f32) -> Self {
        Self {
            min: value,
            max: value,
        }
    }

    pub fn observe(&mut self, value: f32) {
        if value.is_finite() {
            self.min = self.min.min(value);
            self.max = self.max.max(value);
        }
    }

    pub fn axis(&self, current: Axis) -> Axis {
        let span = self.max - self.min;
        if span.is_nan() || span < MIN_GAZE_SPAN {
            return current;
        }
        Axis {
            gain: 2.0 / span,
            offset: (self.max + self.min) / 2.0,
        }
        .sanitized()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remap_scales_and_clamps() {
        assert_eq!(remap(0.5, 0.0, 1.0), 0.5);
        assert!((remap(0.3, 0.2, 0.6) - 0.25).abs() < 1e-6);
        assert_eq!(remap(0.1, 0.2, 0.6), 0.0);
        assert_eq!(remap(0.9, 0.2, 0.6), 1.0);
        assert_eq!(remap(1.4, 0.5, 0.5), 1.0);
        assert_eq!(remap(0.7, 0.9, 0.1), 0.7);
        assert_eq!(remap(f32::NAN, 0.0, 1.0), 0.0);
    }

    #[test]
    fn calibration_widens_to_observed_extremes() {
        let mut ranges = Ranges::identity(2);
        ranges.begin(&[0.4, 0.5]);
        ranges.observe(&[0.2, 0.55]);
        ranges.observe(&[0.6, f32::NAN]);
        assert_eq!(ranges.lower, vec![0.2, 0.5]);
        assert_eq!(ranges.upper, vec![0.6, 0.55]);
        let mut values = [0.4, 0.52];
        ranges.apply(&mut values);
        assert!((values[0] - 0.5).abs() < 1e-6);
        assert!((values[1] - 0.4).abs() < 1e-4);
    }

    #[test]
    fn ranges_are_sanitized_to_length() {
        let ranges = Ranges {
            lower: vec![f32::NAN],
            upper: vec![],
        }
        .sanitized(3);
        assert_eq!(ranges.lower, vec![0.0; 3]);
        assert_eq!(ranges.upper, vec![1.0; 3]);
    }

    #[test]
    fn gaze_axis_centres_and_scales() {
        let axis = Axis {
            gain: 2.0,
            offset: 0.1,
        };
        assert!((axis.apply(0.3) - 0.4).abs() < 1e-6);
        assert_eq!(axis.apply(2.0), 1.0);
        let mut extent = Extent::new(-0.3);
        extent.observe(0.5);
        let calibrated = extent.axis(Axis::default());
        assert!((calibrated.offset - 0.1).abs() < 1e-6);
        assert!((calibrated.gain - 2.5).abs() < 1e-5);
        assert_eq!(Extent::new(0.2).axis(axis), axis);
    }
}
