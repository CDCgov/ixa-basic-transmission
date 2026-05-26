use serde::{Deserialize, Serialize};

/// Per-person infectiousness rate, indexed by time since the person became
/// infectious (τ). Each variant carries its own duration since the two
/// are inseparable in the model: `Constant` needs an explicit duration
/// for `Exp` recovery, and `Empirical`'s duration is derived from the
/// curve's support.
///
/// - `Constant { value, duration }`: fixed rate, recovery is
///   `Exp(1/duration)`.
/// - `Empirical { points }`: piecewise-linear infectiousness curve in
///   τ-space. Rate is 0 outside the anchor range; recovery is
///   deterministic at τ = `points.last().0`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum InfectionRate {
    Constant { value: f64, duration: f64 },
    Empirical { points: Vec<[f64; 2]> },
}

impl InfectionRate {
    /// Rate at elapsed time `τ`. For `Empirical`, linear interpolation
    /// between anchor points; **zero outside the anchor range** (latent
    /// before `points[0].0`, recovered after `points.last().0`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn rate_at(&self, t: f64) -> f64 {
        match self {
            InfectionRate::Constant { value, .. } => *value,
            InfectionRate::Empirical { points } => {
                if points.is_empty() {
                    return 0.0;
                }
                let first = points[0];
                let last = points[points.len() - 1];
                if t < first[0] || t > last[0] {
                    return 0.0;
                }
                let idx = points.partition_point(|p| p[0] <= t);
                if idx == 0 {
                    return first[1];
                }
                if idx >= points.len() {
                    return last[1];
                }
                let lo = points[idx - 1];
                let hi = points[idx];
                let span = hi[0] - lo[0];
                if span == 0.0 {
                    return hi[1];
                }
                let alpha = (t - lo[0]) / span;
                lo[1] + alpha * (hi[1] - lo[1])
            }
        }
    }

    /// Cumulative hazard `Λ(τ) = ∫₀^τ λ(s) ds` where `λ(s) = 0` outside
    /// the anchor range. Used with `inverse_cum_rate` for inverse-CDF
    /// sampling of the next event time in a non-homogeneous Poisson
    /// process. `Constant` is unbounded; `Empirical` saturates at the
    /// total integral past `points.last().0`.
    pub fn cum_rate(&self, t: f64) -> f64 {
        if t <= 0.0 {
            return 0.0;
        }
        match self {
            InfectionRate::Constant { value, .. } => value * t,
            InfectionRate::Empirical { points } => {
                if points.is_empty() {
                    return 0.0;
                }
                let t0 = points[0][0];
                if t <= t0 {
                    return 0.0;
                }
                let mut acc = 0.0;
                for i in 0..points.len() - 1 {
                    let [ti, ri] = points[i];
                    let [tj, rj] = points[i + 1];
                    if t >= tj {
                        acc += 0.5 * (ri + rj) * (tj - ti);
                    } else {
                        let dt = t - ti;
                        let slope = (rj - ri) / (tj - ti);
                        let r_at_t = ri + slope * dt;
                        return acc + 0.5 * (ri + r_at_t) * dt;
                    }
                }
                // t ≥ last anchor: rate is 0 after, so cum stays at total.
                acc
            }
        }
    }

    /// Solves `cum_rate(τ) = c` for `τ ≥ 0`. Returns `None` when `c`
    /// exceeds the schedule's total cumulative hazard (i.e. the curve
    /// has been exhausted — the person is effectively done transmitting).
    pub fn inverse_cum_rate(&self, c: f64) -> Option<f64> {
        if c < 0.0 {
            return None;
        }
        if c == 0.0 {
            return Some(0.0);
        }
        match self {
            InfectionRate::Constant { value, .. } => {
                if *value <= 0.0 {
                    None
                } else {
                    Some(c / value)
                }
            }
            InfectionRate::Empirical { points } => {
                if points.is_empty() {
                    return None;
                }
                // Rate is 0 for τ < points[0].0, so cum stays at 0 there;
                // for c > 0 we always start integrating from the first
                // anchor.
                let mut acc = 0.0;
                for i in 0..points.len() - 1 {
                    let [ti, ri] = points[i];
                    let [tj, rj] = points[i + 1];
                    let seg = 0.5 * (ri + rj) * (tj - ti);
                    if c > acc + seg {
                        acc += seg;
                        continue;
                    }
                    let extra = c - acc;
                    let span = tj - ti;
                    let slope = (rj - ri) / span;
                    // Solve r_i·u + slope/2·u² = extra for u ∈ [0, span].
                    if slope == 0.0 {
                        // ri > 0 here: zero-rate segments have seg = 0,
                        // so we'd only reach this with c == acc, which
                        // implies extra == 0 — almost surely impossible
                        // in the hot path where e ~ Exp(1) is continuous.
                        return Some(ti + extra / ri);
                    }
                    let disc = (ri * ri + 2.0 * slope * extra).max(0.0);
                    let u = (-ri + disc.sqrt()) / slope;
                    return Some(ti + u);
                }
                // c exceeds the curve's total integrated hazard.
                None
            }
        }
    }

    /// Duration of the infectious period. For `Constant` this is the
    /// mean of the `Exp` recovery distribution; for `Empirical` it's
    /// the curve's support (`points.last().0`), at which recovery is
    /// deterministic. Used for expected-R₀ display and reporting.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn duration(&self) -> f64 {
        match self {
            InfectionRate::Constant { duration, .. } => *duration,
            InfectionRate::Empirical { points } => points.last().map_or(0.0, |p| p[0]),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            InfectionRate::Constant { value, duration } => {
                if !value.is_finite() || *value < 0.0 {
                    return Err(format!(
                        "infection_rate value must be a finite non-negative number, got {value}"
                    ));
                }
                if !duration.is_finite() || *duration <= 0.0 {
                    return Err(format!(
                        "infection_rate duration must be a finite positive number, got {duration}"
                    ));
                }
            }
            InfectionRate::Empirical { points } => {
                if points.is_empty() {
                    return Err(
                        "infection_rate empirical schedule must have at least one point"
                            .to_string(),
                    );
                }
                if points[0][0] < 0.0 {
                    return Err(format!(
                        "infection_rate first point time must be non-negative, got {}",
                        points[0][0]
                    ));
                }
                let mut prev_t = f64::NEG_INFINITY;
                for [t, r] in points {
                    if !t.is_finite() {
                        return Err(format!("infection_rate point time must be finite, got {t}"));
                    }
                    if !r.is_finite() || *r < 0.0 {
                        return Err(format!(
                            "infection_rate point rate must be finite and non-negative, got {r}"
                        ));
                    }
                    if *t < prev_t {
                        return Err(format!(
                            "infection_rate points must be sorted by time, got {t} after {prev_t}"
                        ));
                    }
                    prev_t = *t;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_rejects_negative_value() {
        let r = InfectionRate::Constant {
            value: -1.0,
            duration: 3.0,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn constant_rejects_nan_value() {
        let r = InfectionRate::Constant {
            value: f64::NAN,
            duration: 3.0,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn constant_accepts_zero_value() {
        let r = InfectionRate::Constant {
            value: 0.0,
            duration: 3.0,
        };
        r.validate().unwrap();
    }

    #[test]
    fn constant_rejects_zero_duration() {
        let r = InfectionRate::Constant {
            value: 0.5,
            duration: 0.0,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn constant_rejects_infinite_duration() {
        let r = InfectionRate::Constant {
            value: 0.5,
            duration: f64::INFINITY,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn empirical_must_be_non_empty() {
        let r = InfectionRate::Empirical { points: vec![] };
        assert!(r.validate().is_err());
    }

    #[test]
    fn empirical_must_be_sorted() {
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 0.5], [10.0, 0.2], [5.0, 0.1]],
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn empirical_rejects_negative_first_time() {
        let r = InfectionRate::Empirical {
            points: vec![[-1.0, 0.5], [10.0, 0.5]],
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn empirical_rejects_negative_rate() {
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 0.5], [10.0, -0.1]],
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn rate_at_constant() {
        let r = InfectionRate::Constant {
            value: 0.7,
            duration: 1.0,
        };
        assert_eq!(r.rate_at(0.0), 0.7);
        assert_eq!(r.rate_at(100.0), 0.7);
    }

    #[test]
    fn rate_at_empirical_interpolates() {
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 0.2], [10.0, 0.8], [20.0, 0.4]],
        };
        // Outside the anchor range: rate is 0 (latent / recovered).
        assert_eq!(r.rate_at(-5.0), 0.0);
        assert_eq!(r.rate_at(100.0), 0.0);
        // Endpoints.
        assert_eq!(r.rate_at(0.0), 0.2);
        assert_eq!(r.rate_at(10.0), 0.8);
        assert_eq!(r.rate_at(20.0), 0.4);
        // Midpoints.
        assert!((r.rate_at(5.0) - 0.5).abs() < 1e-12);
        assert!((r.rate_at(15.0) - 0.6).abs() < 1e-12);
    }

    #[test]
    fn cum_rate_constant() {
        let r = InfectionRate::Constant {
            value: 0.5,
            duration: 1.0,
        };
        assert_eq!(r.cum_rate(0.0), 0.0);
        assert_eq!(r.cum_rate(10.0), 5.0);
        assert_eq!(r.cum_rate(-1.0), 0.0);
    }

    #[test]
    fn cum_rate_empirical_trapezoidal() {
        // λ(τ) = τ/10 on [0, 10] (linear ramp 0 → 1), then 0 afterwards.
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 0.0], [10.0, 1.0]],
        };
        // ∫₀⁵ (τ/10) dτ = 25/20 = 1.25
        assert!((r.cum_rate(5.0) - 1.25).abs() < 1e-12);
        // ∫₀¹⁰ = 5.0
        assert!((r.cum_rate(10.0) - 5.0).abs() < 1e-12);
        // Past the last anchor the curve is over; cum saturates at 5.0.
        assert!((r.cum_rate(15.0) - 5.0).abs() < 1e-12);
    }

    #[test]
    fn inverse_cum_rate_roundtrips_constant() {
        let r = InfectionRate::Constant {
            value: 0.3,
            duration: 1.0,
        };
        for c in [0.5, 1.0, 7.7] {
            let t = r.inverse_cum_rate(c).unwrap();
            assert!((r.cum_rate(t) - c).abs() < 1e-12);
        }
    }

    #[test]
    fn inverse_cum_rate_roundtrips_empirical_linear() {
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 0.0], [10.0, 2.0]],
        };
        for c in [0.1, 1.0, 5.0, 9.0] {
            let t = r.inverse_cum_rate(c).unwrap();
            assert!((r.cum_rate(t) - c).abs() < 1e-9, "c={c} t={t}");
        }
    }

    #[test]
    fn inverse_cum_rate_roundtrips_empirical_step_down() {
        // High rate then low rate; tests segment-walking.
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 2.0], [5.0, 2.0], [5.001, 0.5], [20.0, 0.5]],
        };
        // Total integral ≈ 2·5 + (2+0.5)/2·0.001 + 0.5·15 ≈ 17.5; check
        // values within that range.
        for c in [0.5, 5.0, 10.0, 15.0, 17.0] {
            let t = r.inverse_cum_rate(c).unwrap();
            assert!((r.cum_rate(t) - c).abs() < 1e-9, "c={c} t={t}");
        }
        // Past the curve's total integral: no valid τ.
        assert_eq!(r.inverse_cum_rate(30.0), None);
    }

    #[test]
    fn cum_rate_peak_shape_evaluates_partial_segments() {
        // Mirrors ixa-epi-isolation's `test_cum_rate_eval`: a tent-shaped
        // schedule that exercises partial-segment integration on both the
        // ramp-up and ramp-down sides.
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [3.0, 1.0], [4.0, 0.0]],
        };
        // cum_rate(1.5): full segment [0,1] = 0.5, partial [1,1.5] with
        // λ(1.5)=1.5 → trapezoid (1+1.5)/2 · 0.5 = 0.625. Total 1.125.
        assert!((r.cum_rate(1.5) - 1.125).abs() < 1e-12);
        // cum_rate(2.5): segments [0,1] + [1,2] = 0.5 + 1.5 = 2.0, then
        // partial [2,2.5] with λ(2.5)=1.5 → (2+1.5)/2 · 0.5 = 0.875.
        // Total 2.875.
        assert!((r.cum_rate(2.5) - 2.875).abs() < 1e-12);
    }

    #[test]
    fn inverse_cum_rate_with_plateau_walks_forward() {
        // Mirrors `test_inverse_cum_rate_plateaus`. A schedule with a
        // zero-rate stretch in the middle: cumulative is flat across it.
        // `inverse_cum_rate(c)` at the plateau's value must return the
        // earliest time it's reached, and just past it should jump past
        // the plateau.
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 1.0], [1.0, 1.0], [2.0, 0.0], [3.0, 0.0], [4.0, 1.0]],
        };
        // cum at anchors: 0, 1.0, 1.5, 1.5, 2.0
        assert!((r.cum_rate(2.0) - 1.5).abs() < 1e-12);
        assert!((r.cum_rate(3.0) - 1.5).abs() < 1e-12);
        // At c=1.5 the inverse should land at the *earliest* t where
        // cum_rate equals 1.5, i.e. t = 2.0 (the start of the plateau).
        let t_at_15 = r.inverse_cum_rate(1.5).unwrap();
        assert!(t_at_15 - 2.0 < 1e-12 && t_at_15 >= 2.0 - 1e-12);
        // Just above 1.5 should be on the next ramp (t > 3.0).
        let t_above = r.inverse_cum_rate(1.6).unwrap();
        assert!(t_above > 3.0, "expected past plateau, got {t_above}");
    }

    #[test]
    fn inverse_cum_rate_returns_none_when_unattainable() {
        // Constant zero, or empirical that flatlines at zero past last anchor.
        let zero = InfectionRate::Constant {
            value: 0.0,
            duration: 1.0,
        };
        assert_eq!(zero.inverse_cum_rate(1.0), None);

        let trailing_zero = InfectionRate::Empirical {
            points: vec![[0.0, 1.0], [5.0, 0.0]],
        };
        // Total area on [0, 5] = 0.5 · 1.0 · 5.0 = 2.5; anything beyond is unreachable.
        assert_eq!(trailing_zero.inverse_cum_rate(3.0), None);
        assert!(trailing_zero.inverse_cum_rate(1.0).is_some());
    }

    #[test]
    fn empirical_serde_roundtrip_json() {
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 0.5], [20.0, 0.1]],
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: InfectionRate = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
        // Sanity-check the wire shape.
        assert!(json.contains("\"type\":\"empirical\""));
    }

    #[test]
    fn constant_serde_roundtrip_json() {
        let r = InfectionRate::Constant {
            value: 0.42,
            duration: 3.0,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(
            json,
            "{\"type\":\"constant\",\"value\":0.42,\"duration\":3.0}"
        );
        let back: InfectionRate = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
