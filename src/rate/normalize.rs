//! Normalize an `InfectionRate` so its `scale` field equals the expected
//! R₀ under random mixing. The model calls this once at the entry to
//! `run` / `setup_only`, so every downstream consumer (UI, benchmarks,
//! future external embedders) gets the same `scale = R₀` contract.
//!
//! The contract:
//! - **Empirical**: rescale points so `∫ shape(τ) dτ = 1`. Then
//!   `R₀ = ∫ scale · shape = scale · 1 = scale`.
//! - **Library**: rescale every curve by the *same* factor so the
//!   *mean* curve has area 1 (per-curve heterogeneity preserved). Then
//!   `E_i[∫ scale · shape_i] = scale · 1 = scale`.
//! - **Constant**: no-op — R₀ is already `value · duration` and the
//!   user controls both directly.

use super::{empirical_cum_rate, empirical_curve_duration, InfectionRate};

/// See module docs. Idempotent; no-op on degenerate (empty / zero-area)
/// inputs — those hit `cum_rate`'s exhausted-curve path during setup
/// and produce no transmission, which is the right behavior.
pub fn normalize_to_r0(rate: &mut InfectionRate) {
    match rate {
        InfectionRate::Constant { .. } => {}
        // `Parametric` is lowered to `Empirical` (`materialize_parametric`)
        // before this is called in `model::run` / `setup_only`, so it is
        // already an `Empirical` curve by the time we normalize. Left as a
        // no-op for the rare standalone call on a still-parametric rate.
        InfectionRate::Parametric { .. } => {}
        InfectionRate::Empirical { points, .. } => {
            let area = empirical_cum_rate(points, empirical_curve_duration(points));
            if !area.is_finite() || area <= 0.0 {
                return;
            }
            for p in points.iter_mut() {
                p[1] /= area;
            }
        }
        InfectionRate::Library { rates, .. } => {
            if rates.is_empty() {
                return;
            }
            let total: f64 = rates
                .iter()
                .map(|r| empirical_cum_rate(r, empirical_curve_duration(r)))
                .sum();
            let mean = total / rates.len() as f64;
            if !mean.is_finite() || mean <= 0.0 {
                return;
            }
            for r in rates.iter_mut() {
                for p in r.iter_mut() {
                    p[1] /= mean;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rate::{EmpiricalRate, InfectiousnessRateFn};

    /// Trapezoidal area for a single curve (test-only sanity helper —
    /// mirrors what `normalize_to_r0` measures, kept separate so the
    /// tests don't lean on the same code path they're verifying).
    fn trapezoid_area(points: &[[f64; 2]]) -> f64 {
        let mut acc = 0.0;
        for i in 0..points.len().saturating_sub(1) {
            let [ti, ri] = points[i];
            let [tj, rj] = points[i + 1];
            acc += 0.5 * (ri + rj) * (tj - ti);
        }
        acc
    }

    #[test]
    fn empirical_area_becomes_one() {
        let mut r = InfectionRate::Empirical {
            points: vec![[0.0, 0.0], [2.0, 1.0], [4.0, 1.2], [6.0, 0.4], [8.0, 0.0]],
            scale: 3.0,
        };
        normalize_to_r0(&mut r);
        match &r {
            InfectionRate::Empirical { points, scale } => {
                assert!((trapezoid_area(points) - 1.0).abs() < 1e-12);
                // Scale must be left alone — that's the contract.
                assert_eq!(*scale, 3.0);
            }
            _ => panic!("variant changed"),
        }
    }

    #[test]
    fn empirical_preserves_shape_ratios() {
        // After normalization the relative magnitudes between points
        // must be unchanged — only the absolute scale shifts.
        let raw = vec![[0.0, 0.0], [2.0, 1.0], [4.0, 1.2], [6.0, 0.4], [8.0, 0.0]];
        let mut r = InfectionRate::Empirical {
            points: raw.clone(),
            scale: 1.0,
        };
        normalize_to_r0(&mut r);
        match &r {
            InfectionRate::Empirical { points, .. } => {
                // peak/early ratio: 1.2 / 1.0 = 1.2 (preserved exactly).
                assert!((points[2][1] / points[1][1] - 1.2).abs() < 1e-12);
                // tail/peak ratio: 0.4 / 1.2.
                assert!((points[3][1] / points[2][1] - 0.4 / 1.2).abs() < 1e-12);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn library_mean_area_becomes_one() {
        let mut r = InfectionRate::Library {
            rates: vec![
                vec![[0.0, 0.0], [1.0, 2.0], [2.0, 0.0]], // area = 2
                vec![[0.0, 0.0], [4.0, 1.0], [8.0, 0.0]], // area = 4
                vec![[0.0, 1.0], [6.0, 1.0]],             // area = 6
            ],
            scale: 3.0,
        };
        normalize_to_r0(&mut r);
        match &r {
            InfectionRate::Library { rates, scale } => {
                let total: f64 = rates.iter().map(|c| trapezoid_area(c)).sum();
                let mean = total / rates.len() as f64;
                assert!((mean - 1.0).abs() < 1e-12, "mean area = {mean}");
                assert_eq!(*scale, 3.0);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn library_preserves_per_curve_heterogeneity() {
        // Three curves with raw areas in ratio 2 : 4 : 6 = 1 : 2 : 3.
        // Normalization divides every curve by the SAME constant (the
        // mean area), so the ratio must stay 1 : 2 : 3.
        let mut r = InfectionRate::Library {
            rates: vec![
                vec![[0.0, 0.0], [1.0, 2.0], [2.0, 0.0]], // area = 2
                vec![[0.0, 0.0], [4.0, 1.0], [8.0, 0.0]], // area = 4
                vec![[0.0, 1.0], [6.0, 1.0]],             // area = 6
            ],
            scale: 1.0,
        };
        normalize_to_r0(&mut r);
        match &r {
            InfectionRate::Library { rates, .. } => {
                let areas: Vec<f64> = rates.iter().map(|c| trapezoid_area(c)).collect();
                assert!((areas[1] / areas[0] - 2.0).abs() < 1e-12);
                assert!((areas[2] / areas[0] - 3.0).abs() < 1e-12);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn constant_is_noop() {
        // For Constant, R₀ = value × duration directly — no points to
        // rescale. The struct must be byte-identical after normalize.
        let mut r = InfectionRate::Constant {
            value: 0.5,
            duration: 6.0,
        };
        let before = r.clone();
        normalize_to_r0(&mut r);
        assert_eq!(r, before);
    }

    /// Approximate point-wise equality between two empirical-shape
    /// curves — second-pass normalize divides by `1 ± ε` (not exact 1)
    /// so we compare with a tolerance, not `assert_eq!`.
    fn assert_points_approx(a: &[[f64; 2]], b: &[[f64; 2]]) {
        assert_eq!(a.len(), b.len(), "length mismatch");
        for (i, (ap, bp)) in a.iter().zip(b.iter()).enumerate() {
            assert!(
                (ap[0] - bp[0]).abs() < 1e-12 && (ap[1] - bp[1]).abs() < 1e-12,
                "point #{i}: {ap:?} != {bp:?}"
            );
        }
    }

    #[test]
    fn idempotent_empirical() {
        let mut r = InfectionRate::Empirical {
            points: vec![[0.0, 0.0], [2.0, 1.0], [4.0, 1.2], [6.0, 0.4], [8.0, 0.0]],
            scale: 2.5,
        };
        normalize_to_r0(&mut r);
        let once = r.clone();
        normalize_to_r0(&mut r);
        // Second pass divides by area ≈ 1 (modulo floating-point drift)
        // so the curve is unchanged within tolerance.
        match (&r, &once) {
            (
                InfectionRate::Empirical {
                    points: a,
                    scale: sa,
                },
                InfectionRate::Empirical {
                    points: b,
                    scale: sb,
                },
            ) => {
                assert_points_approx(a, b);
                assert_eq!(sa, sb);
            }
            _ => panic!("variant changed"),
        }
    }

    #[test]
    fn idempotent_library() {
        let mut r = InfectionRate::Library {
            rates: vec![
                vec![[0.0, 0.0], [1.0, 2.0], [2.0, 0.0]],
                vec![[0.0, 0.0], [4.0, 1.0], [8.0, 0.0]],
            ],
            scale: 1.5,
        };
        normalize_to_r0(&mut r);
        let once = r.clone();
        normalize_to_r0(&mut r);
        match (&r, &once) {
            (
                InfectionRate::Library {
                    rates: a,
                    scale: sa,
                },
                InfectionRate::Library {
                    rates: b,
                    scale: sb,
                },
            ) => {
                assert_eq!(a.len(), b.len());
                for (ca, cb) in a.iter().zip(b.iter()) {
                    assert_points_approx(ca, cb);
                }
                assert_eq!(sa, sb);
            }
            _ => panic!("variant changed"),
        }
    }

    #[test]
    fn empty_library_is_noop() {
        let mut r = InfectionRate::Library {
            rates: vec![],
            scale: 1.0,
        };
        let before = r.clone();
        normalize_to_r0(&mut r);
        assert_eq!(r, before);
    }

    #[test]
    fn zero_area_empirical_is_noop() {
        // A curve that's flat zero has area 0 — no meaningful way to
        // normalize. Leave it alone; the model's exhausted-curve path
        // handles it gracefully (no transmission).
        let mut r = InfectionRate::Empirical {
            points: vec![[0.0, 0.0], [5.0, 0.0]],
            scale: 1.0,
        };
        let before = r.clone();
        normalize_to_r0(&mut r);
        assert_eq!(r, before);
    }

    #[test]
    fn effective_rate_total_equals_scale_after_normalize_empirical() {
        // Integration check: after normalization, the integrated hazard
        // over the curve's full support equals `scale` exactly (i.e. R₀
        // under random mixing).
        let mut r = InfectionRate::Empirical {
            points: vec![[0.0, 0.0], [2.0, 1.0], [4.0, 1.2], [6.0, 0.4], [8.0, 0.0]],
            scale: 3.0,
        };
        normalize_to_r0(&mut r);
        let (points, scale) = match &r {
            InfectionRate::Empirical { points, scale } => (points.clone(), *scale),
            _ => panic!(),
        };
        let total = EmpiricalRate::new(points, scale).max_cum_rate();
        assert!(
            (total - scale).abs() < 1e-12,
            "total = {total}, scale = {scale}"
        );
    }

    #[test]
    fn parametric_lowered_then_normalized_has_area_one() {
        // Lowering a Parametric to its Empirical grid then normalizing must
        // give a unit-area curve, so `scale` is the expected R₀ — same
        // contract as a hand-entered Empirical curve.
        use crate::rate::{materialize_parametric, ParametricDist};
        let mut r = InfectionRate::Parametric {
            dist: ParametricDist::Gamma {
                shape: 3.0,
                scale: 2.0,
            },
            scale: 4.0,
        };
        materialize_parametric(&mut r);
        normalize_to_r0(&mut r);
        let (points, scale) = match &r {
            InfectionRate::Empirical { points, scale } => (points.clone(), *scale),
            _ => panic!("should be Empirical after lowering"),
        };
        assert!((trapezoid_area(&points) - 1.0).abs() < 1e-12);
        // And the integrated effective hazard equals `scale` (= R₀).
        assert!((EmpiricalRate::new(points, scale).max_cum_rate() - scale).abs() < 1e-12);
    }

    #[test]
    fn effective_rate_mean_total_equals_scale_after_normalize_library() {
        // Same check, library variant: averaging the per-curve totals
        // across the library must equal `scale`.
        let mut r = InfectionRate::Library {
            rates: vec![
                vec![[0.0, 0.0], [1.0, 2.0], [2.0, 0.0]],
                vec![[0.0, 0.0], [4.0, 1.0], [8.0, 0.0]],
                vec![[0.0, 1.0], [6.0, 1.0]],
            ],
            scale: 3.0,
        };
        normalize_to_r0(&mut r);
        let (rates, scale) = match &r {
            InfectionRate::Library { rates, scale } => (rates.clone(), *scale),
            _ => panic!(),
        };
        let mean_total: f64 = rates
            .iter()
            .map(|c| EmpiricalRate::new(c.clone(), scale).max_cum_rate())
            .sum::<f64>()
            / rates.len() as f64;
        assert!(
            (mean_total - scale).abs() < 1e-12,
            "mean_total = {mean_total}, scale = {scale}"
        );
    }
}
