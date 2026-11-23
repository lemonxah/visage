use std::f32::consts::PI;

use glam::Vec3;

const DEFAULT_SCALE: f32 = PI / 6.0;
const MIN_SCALE: f32 = PI / 12.0;
const MAX_SCALE: f32 = PI / 3.0;
const MAX_OFFSET: f32 = PI / 12.0;
const FIT_DECAY: f32 = 0.999;
const FIT_MIN_DEFLECTION: f32 = 0.1;

pub(super) struct Vergence {
    fit_num: f32,
    fit_den: f32,
}

impl Vergence {
    pub fn new() -> Self {
        Self {
            fit_num: DEFAULT_SCALE,
            fit_den: 1.0,
        }
    }

    fn scale(&self) -> f32 {
        (self.fit_num / self.fit_den).clamp(MIN_SCALE, MAX_SCALE)
    }

    pub fn split(&mut self, combined: Vec3, left_x: f32, right_x: f32) -> [Vec3; 2] {
        if !left_x.is_finite() || !right_x.is_finite() || !combined.is_finite() {
            return [combined; 2];
        }

        let mean_x = (left_x + right_x) * 0.5;
        if mean_x.abs() >= FIT_MIN_DEFLECTION {
            let gaze_right = -combined.y;
            self.fit_num = self.fit_num * FIT_DECAY + gaze_right * mean_x;
            self.fit_den = self.fit_den * FIT_DECAY + mean_x * mean_x;
        }

        let inward = (self.scale() * (left_x - right_x) * 0.5).clamp(-MAX_OFFSET, MAX_OFFSET);
        let mut left = combined;
        let mut right = combined;
        left.y -= inward;
        right.y += inward;
        [left, right]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::vec3;

    #[test]
    fn identical_eyes_stay_parallel() {
        let combined = vec3(0.1, -0.15, 0.0);
        let eyes = Vergence::new().split(combined, 0.3, 0.3);
        assert_eq!(eyes, [combined; 2]);
    }

    #[test]
    fn converging_eyes_turn_inward_symmetrically() {
        let eyes = Vergence::new().split(Vec3::ZERO, 0.2, -0.2);
        assert!(eyes[0].y < 0.0);
        assert!(eyes[1].y > 0.0);
        assert!((eyes[0].y + eyes[1].y).abs() < 1e-6);
        assert!((eyes[1].y - DEFAULT_SCALE * 0.2).abs() < 1e-6);
    }

    #[test]
    fn offset_is_bounded() {
        let eyes = Vergence::new().split(Vec3::ZERO, 1.0, -1.0);
        assert_eq!(eyes[1].y, MAX_OFFSET);
        assert_eq!(eyes[0].y, -MAX_OFFSET);
    }

    #[test]
    fn pitch_and_roll_are_untouched() {
        let combined = vec3(0.2, 0.0, 0.05);
        let eyes = Vergence::new().split(combined, 0.2, -0.2);
        assert_eq!((eyes[0].x, eyes[0].z), (combined.x, combined.z));
        assert_eq!((eyes[1].x, eyes[1].z), (combined.x, combined.z));
    }

    #[test]
    fn scale_follows_combined_gaze() {
        let mut vergence = Vergence::new();
        let scale = 0.8;
        for step in 0..4000 {
            let x = ((step % 200) as f32 / 100.0 - 1.0) * 0.6;
            vergence.split(vec3(0.0, -scale * x, 0.0), x, x);
        }
        assert!((vergence.scale() - scale).abs() < 0.05);
    }

    #[test]
    fn non_finite_input_leaves_gaze_and_fit_alone() {
        let mut vergence = Vergence::new();
        let combined = vec3(0.0, 0.3, 0.0);
        assert_eq!(vergence.split(combined, f32::NAN, 0.2), [combined; 2]);
        assert_eq!(vergence.scale(), DEFAULT_SCALE);
    }
}
