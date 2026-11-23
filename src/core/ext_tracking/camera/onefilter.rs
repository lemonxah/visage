use std::{
    f32::consts::TAU,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

const RESET_AFTER: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterParams {
    pub enabled: bool,
    pub min_cutoff: f32,
    pub beta: f32,
    pub d_cutoff: f32,
}

impl Default for FilterParams {
    fn default() -> Self {
        Self {
            enabled: true,
            min_cutoff: 0.5,
            beta: 3.0,
            d_cutoff: 1.0,
        }
    }
}

impl FilterParams {
    pub fn sanitized(self) -> Self {
        let defaults = Self::default();
        let positive = |v: f32, fallback: f32| {
            if v.is_finite() && v > 0.0 {
                v.min(100.0)
            } else {
                fallback
            }
        };
        Self {
            enabled: self.enabled,
            min_cutoff: positive(self.min_cutoff, defaults.min_cutoff),
            beta: if self.beta.is_finite() {
                self.beta.clamp(0.0, 100.0)
            } else {
                defaults.beta
            },
            d_cutoff: positive(self.d_cutoff, defaults.d_cutoff),
        }
    }
}

fn alpha(cutoff: f32, dt: f32) -> f32 {
    let tau = 1.0 / (TAU * cutoff);
    1.0 / (1.0 + tau / dt)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OneEuro {
    value: Option<f32>,
    derivative: f32,
}

impl OneEuro {
    pub fn filter(&mut self, value: f32, dt: f32, params: &FilterParams) -> f32 {
        let Some(previous) = self.value else {
            self.value = Some(value);
            self.derivative = 0.0;
            return value;
        };
        let dt = dt.max(1e-4);
        let raw_derivative = (value - previous) / dt;
        let a_d = alpha(params.d_cutoff, dt);
        self.derivative = a_d * raw_derivative + (1.0 - a_d) * self.derivative;
        let cutoff = params.min_cutoff + params.beta * self.derivative.abs();
        let a = alpha(cutoff, dt);
        let filtered = a * value + (1.0 - a) * previous;
        self.value = Some(filtered);
        filtered
    }

    pub fn reset(&mut self) {
        self.value = None;
        self.derivative = 0.0;
    }
}

pub struct OneEuroBank {
    filters: Vec<OneEuro>,
    last: Option<Instant>,
}

impl OneEuroBank {
    pub fn new(count: usize) -> Self {
        Self {
            filters: vec![OneEuro::default(); count],
            last: None,
        }
    }

    pub fn filter(&mut self, values: &mut [f32], at: Instant, params: &FilterParams) {
        let dt = self.last.map(|last| at.saturating_duration_since(last));
        self.last = Some(at);
        if !params.enabled || dt.is_none_or(|dt| dt > RESET_AFTER) {
            for filter in self.filters.iter_mut() {
                filter.reset();
            }
            if !params.enabled {
                return;
            }
        }
        let dt = dt.map_or(1.0 / 60.0, |dt| dt.as_secs_f32());
        for (value, filter) in values.iter_mut().zip(self.filters.iter_mut()) {
            *value = filter.filter(*value, dt, params);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_passes_through_and_constant_input_stays_put() {
        let params = FilterParams::default();
        let mut filter = OneEuro::default();
        assert_eq!(filter.filter(0.4, 1.0 / 60.0, &params), 0.4);
        for _ in 0..100 {
            assert!((filter.filter(0.4, 1.0 / 60.0, &params) - 0.4).abs() < 1e-6);
        }
    }

    #[test]
    fn slow_noise_is_damped_and_fast_moves_are_followed() {
        let params = FilterParams::default();
        let mut filter = OneEuro::default();
        filter.filter(0.0, 1.0 / 60.0, &params);
        let jitter = filter.filter(0.02, 1.0 / 60.0, &params);
        assert!(jitter > 0.0 && jitter < 0.01, "{jitter}");

        let mut slow = OneEuro::default();
        let mut fast = OneEuro::default();
        let lazy = FilterParams {
            beta: 0.0,
            ..params
        };
        slow.filter(0.0, 1.0 / 60.0, &lazy);
        fast.filter(0.0, 1.0 / 60.0, &params);
        let mut slow_value = 0.0;
        let mut fast_value = 0.0;
        for _ in 0..5 {
            slow_value = slow.filter(1.0, 1.0 / 60.0, &lazy);
            fast_value = fast.filter(1.0, 1.0 / 60.0, &params);
        }
        assert!(fast_value > slow_value);
        assert!(fast_value > 0.5, "{fast_value}");
    }

    #[test]
    fn alpha_matches_the_published_formula() {
        let dt = 1.0 / 60.0;
        let tau = 1.0 / (2.0 * std::f32::consts::PI * 1.0);
        assert!((alpha(1.0, dt) - 1.0 / (1.0 + tau / dt)).abs() < 1e-7);
    }

    #[test]
    fn bank_resets_after_a_gap_and_when_disabled() {
        let params = FilterParams::default();
        let mut bank = OneEuroBank::new(1);
        let start = Instant::now();
        let mut values = [0.0];
        bank.filter(&mut values, start, &params);
        let mut values = [1.0];
        bank.filter(&mut values, start + Duration::from_secs(5), &params);
        assert_eq!(values[0], 1.0);
        let disabled = FilterParams {
            enabled: false,
            ..params
        };
        let mut values = [0.25];
        bank.filter(&mut values, start + Duration::from_millis(5016), &disabled);
        assert_eq!(values[0], 0.25);
    }
}
