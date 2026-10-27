use anyhow::{ensure, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct StateError {
    pub name: String,
    pub max_error: f64,
    pub rms_error: f64,
    pub normalized_max_error: f64,
    pub normalized_rms_error: f64,
    pub legacy_element_scaled_max: f64,
    pub tensor_pass: bool,
}

pub fn state_error(name: &str, actual: &[f32], reference: &[f32]) -> Result<StateError> {
    ensure!(
        actual.len() == reference.len() && !actual.is_empty(),
        "State shape mismatch or empty state"
    );
    let (mut peak, mut squares, mut ref_peak, mut ref_squares, mut legacy) =
        (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
    for (&a, &b) in actual.iter().zip(reference) {
        ensure!(a.is_finite() && b.is_finite(), "Nonfinite state");
        let (a, b) = (f64::from(a), f64::from(b));
        let difference = (a - b).abs();
        peak = peak.max(difference);
        squares += difference * difference;
        ref_peak = ref_peak.max(b.abs());
        ref_squares += b * b;
        legacy = legacy.max(difference / (1e-4 + b.abs() * 1e-4));
    }
    let rms = (squares / actual.len() as f64).sqrt();
    let normalized_rms = rms / (ref_squares / actual.len() as f64).sqrt().max(1.0);
    let normalized_max = peak / ref_peak.max(1.0);
    Ok(StateError {
        name: name.to_owned(),
        max_error: peak,
        rms_error: rms,
        normalized_max_error: normalized_max,
        normalized_rms_error: normalized_rms,
        legacy_element_scaled_max: legacy,
        tensor_pass: normalized_rms <= 1e-4 && normalized_max <= 1e-3,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_reference_does_not_amplify_noise() {
        let result = state_error("silent", &[0.00001, 0.00001], &[0.0, 0.0]).unwrap();
        assert!(result.tensor_pass);
        assert!((result.normalized_rms_error - 0.00001).abs() < 1e-10);
    }

    #[test]
    fn peak_limit_detects_sparse_corruption_despite_small_rms() {
        let mut a = vec![0.0; 1024];
        a[0] = 0.002;
        let result = state_error("sparse", &a, &vec![0.0; 1024]).unwrap();
        assert!(result.normalized_rms_error < 1e-4);
        assert!(!result.tensor_pass);
    }

    #[test]
    fn rejects_nonfinite_and_mismatched_state() {
        assert!(state_error("invalid", &[f32::NAN], &[0.0]).is_err());
        assert!(state_error("invalid", &[0.0], &[f32::INFINITY]).is_err());
        assert!(state_error("invalid", &[0.0], &[]).is_err());
        assert!(state_error("invalid", &[], &[]).is_err());
    }
}
