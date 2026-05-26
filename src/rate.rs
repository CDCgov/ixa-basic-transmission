use serde::{Deserialize, Serialize};

/// Per-person infectiousness rate, indexed by time since the person became
/// infectious (τ). Each variant carries its own duration since the two
/// are inseparable in the model: `Constant` needs an explicit duration
/// for `Exp` recovery, and `Empirical`'s duration is derived from the
/// curve's support.
///
/// - `Constant { value, duration }`: fixed rate, recovery is
///   `Exp(1/duration)`. `value` is an absolute hazard rate.
/// - `Empirical { points, scale }`: piecewise-linear infectiousness curve
///   in τ-space. Rate is 0 outside the anchor range; recovery is
///   deterministic at τ = `points.last().0`. **The point values are
///   relative hazards** (in the ixa-epi-isolation sense) — `scale`
///   converts them to absolute rates by multiplication. Defaults to 1.0
///   so simple curves act as absolute rates if the modeler hasn't
///   calibrated.
/// - `Library { rates, scale }`: population of empirical curves with a
///   single shared scale. Each person gets one curve assigned at setup
///   and keeps it for their whole infectious period (per-person
///   heterogeneity). Empty curve list is rejected.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum InfectionRate {
    Constant {
        value: f64,
        duration: f64,
    },
    Empirical {
        points: Vec<[f64; 2]>,
        #[serde(default = "default_scale")]
        scale: f64,
    },
    Library {
        rates: Vec<Vec<[f64; 2]>>,
        #[serde(default = "default_scale")]
        scale: f64,
    },
}

fn default_scale() -> f64 {
    1.0
}

/// Cumulative hazard `Λ(τ)` for a single piecewise-linear empirical
/// curve. Hot-path helper: takes a slice so the caller can avoid
/// allocating or cloning when iterating per-event.
#[inline]
pub fn empirical_cum_rate(points: &[[f64; 2]], t: f64) -> f64 {
    if t <= 0.0 || points.is_empty() {
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
    acc
}

/// Inverse cumulative hazard for a single piecewise-linear empirical
/// curve. `None` when `c` exceeds the curve's total integrated hazard.
/// Hot-path helper (called once per infection attempt).
#[inline]
pub fn empirical_inverse_cum_rate(points: &[[f64; 2]], c: f64) -> Option<f64> {
    if c < 0.0 {
        return None;
    }
    if c == 0.0 {
        return Some(0.0);
    }
    if points.is_empty() {
        return None;
    }
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
        if slope == 0.0 {
            return Some(ti + extra / ri);
        }
        let disc = (ri * ri + 2.0 * slope * extra).max(0.0);
        let u = (-ri + disc.sqrt()) / slope;
        return Some(ti + u);
    }
    None
}

/// Curve support — the deterministic recovery time τ for an empirical
/// schedule. `0.0` if the curve is empty.
#[inline]
pub fn empirical_curve_duration(points: &[[f64; 2]]) -> f64 {
    points.last().map_or(0.0, |p| p[0])
}

/// Piecewise-linear empirical curve with precomputed cumulative hazard
/// at each anchor (`cum[i] = Λ(points[i].0)`, `cum[0] = 0`). Built once
/// at setup; the hot path uses `cum_rate`/`inverse_cum_rate` on this
/// type so it skips the per-segment trapezoid multiply-add accumulation
/// the slice-based helpers do every call.
#[derive(Clone, Debug)]
pub struct Curve {
    pub points: Vec<[f64; 2]>,
    pub cum: Vec<f64>,
}

impl Curve {
    pub fn new(points: Vec<[f64; 2]>) -> Self {
        let mut cum = Vec::with_capacity(points.len());
        cum.push(0.0);
        let mut acc = 0.0;
        for i in 1..points.len() {
            let [ti, ri] = points[i - 1];
            let [tj, rj] = points[i];
            acc += 0.5 * (ri + rj) * (tj - ti);
            cum.push(acc);
        }
        Self { points, cum }
    }

    #[inline]
    pub fn duration(&self) -> f64 {
        empirical_curve_duration(&self.points)
    }

    /// `Λ(τ)`. Walks segments to find the one containing `t`, then adds
    /// one partial trapezoid — same as `empirical_cum_rate` but skips
    /// the running accumulation since each anchor's cumulative is
    /// already in `cum`.
    #[inline]
    pub fn cum_rate(&self, t: f64) -> f64 {
        let points = &self.points;
        if t <= 0.0 || points.is_empty() {
            return 0.0;
        }
        if t <= points[0][0] {
            return 0.0;
        }
        for i in 0..points.len() - 1 {
            let [ti, ri] = points[i];
            let [tj, rj] = points[i + 1];
            if t < tj {
                let dt = t - ti;
                let slope = (rj - ri) / (tj - ti);
                let r_at_t = ri + slope * dt;
                return self.cum[i] + 0.5 * (ri + r_at_t) * dt;
            }
        }
        *self.cum.last().unwrap()
    }

    /// Solves `cum_rate(τ) = c`. Walks segments until `c` lands in
    /// `[cum[i], cum[i+1]]`, then solves the per-segment quadratic on
    /// the leftover — same math as `empirical_inverse_cum_rate` but
    /// without re-summing per-segment trapezoids.
    #[inline]
    pub fn inverse_cum_rate(&self, c: f64) -> Option<f64> {
        if c < 0.0 {
            return None;
        }
        if c == 0.0 {
            return Some(0.0);
        }
        let points = &self.points;
        if points.is_empty() {
            return None;
        }
        for i in 0..points.len() - 1 {
            if c > self.cum[i + 1] {
                continue;
            }
            let [ti, ri] = points[i];
            let [tj, rj] = points[i + 1];
            let extra = c - self.cum[i];
            let span = tj - ti;
            let slope = (rj - ri) / span;
            if slope == 0.0 {
                return Some(ti + extra / ri);
            }
            let disc = (ri * ri + 2.0 * slope * extra).max(0.0);
            let u = (-ri + disc.sqrt()) / slope;
            return Some(ti + u);
        }
        None
    }
}

/// Linear interpolation of a piecewise-linear curve at τ. Zero outside
/// the anchor range. Mainly used by tests / display code.
#[inline]
pub fn rate_at_curve(points: &[[f64; 2]], t: f64) -> f64 {
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

/// Per-person effective rate. Resolved from `InfectionRate` plus any
/// per-person state (a `Library` person's assigned curve). Once
/// resolved, inverse-CDF sampling needs only this primitive — no
/// further variant dispatch — so the transmission loop is one branch
/// instead of three.
#[derive(Debug, Clone, Copy)]
pub enum EffectiveRate<'a> {
    Constant { value: f64 },
    Empirical { curve: &'a Curve, scale: f64 },
}

impl EffectiveRate<'_> {
    /// `Λ(τ)` for the per-person effective rate.
    #[inline]
    pub fn cum_rate(&self, t: f64) -> f64 {
        match *self {
            EffectiveRate::Constant { value } => {
                if t > 0.0 {
                    value * t
                } else {
                    0.0
                }
            }
            EffectiveRate::Empirical { curve, scale } => scale * curve.cum_rate(t),
        }
    }

    /// Solves `cum_rate(τ) = c`. `None` for `c < 0`, for a non-positive
    /// or non-finite constant rate, or when `c` exceeds the empirical
    /// curve's total integrated hazard.
    #[inline]
    pub fn inverse_cum_rate(&self, c: f64) -> Option<f64> {
        if c < 0.0 {
            return None;
        }
        if c == 0.0 {
            return Some(0.0);
        }
        match *self {
            EffectiveRate::Constant { value } => {
                if !value.is_finite() || value <= 0.0 {
                    None
                } else {
                    Some(c / value)
                }
            }
            EffectiveRate::Empirical { curve, scale } => {
                if !scale.is_finite() || scale <= 0.0 {
                    return None;
                }
                curve.inverse_cum_rate(c / scale)
            }
        }
    }
}

fn validate_scale(scale: f64) -> Result<(), String> {
    if !scale.is_finite() || scale < 0.0 {
        return Err(format!(
            "infection_rate scale must be a finite non-negative number, got {scale}"
        ));
    }
    Ok(())
}

fn validate_empirical_points(points: &[[f64; 2]]) -> Result<(), String> {
    if points.is_empty() {
        return Err("infection_rate empirical schedule must have at least one point".to_string());
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
    Ok(())
}

impl InfectionRate {
    /// Rate at elapsed time `τ`. For `Empirical`, linear interpolation
    /// between anchor points; **zero outside the anchor range** (latent
    /// before `points[0].0`, recovered after `points.last().0`).
    /// For `Library`, returns 0 — there is no single curve; use
    /// `rate_at_for_curve` with a specific curve from the library.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn rate_at(&self, t: f64) -> f64 {
        match self {
            InfectionRate::Constant { value, .. } => *value,
            InfectionRate::Empirical { points, scale } => scale * rate_at_curve(points, t),
            InfectionRate::Library { .. } => 0.0,
        }
    }

    /// Cumulative hazard `Λ(τ) = ∫₀^τ λ(s) ds` where `λ(s) = 0` outside
    /// the anchor range. Used with `inverse_cum_rate` for inverse-CDF
    /// sampling of the next event time in a non-homogeneous Poisson
    /// process. `Constant` is unbounded; `Empirical` saturates at the
    /// total integral past `points.last().0`. `Library` panics — the
    /// caller must dispatch through the per-person assigned curve.
    pub fn cum_rate(&self, t: f64) -> f64 {
        match self {
            InfectionRate::Constant { value, .. } if t > 0.0 => value * t,
            InfectionRate::Constant { .. } => 0.0,
            InfectionRate::Empirical { points, scale } => scale * empirical_cum_rate(points, t),
            InfectionRate::Library { .. } => {
                panic!("cum_rate called on Library variant; use the per-person curve")
            }
        }
    }

    /// Solves `cum_rate(τ) = c` for `τ ≥ 0`. Returns `None` when `c`
    /// exceeds the schedule's total cumulative hazard (i.e. the curve
    /// has been exhausted — the person is effectively done transmitting).
    /// `Library` panics — see `cum_rate`.
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
            InfectionRate::Empirical { points, scale } => {
                if *scale <= 0.0 {
                    return None;
                }
                empirical_inverse_cum_rate(points, c / scale)
            }
            InfectionRate::Library { .. } => {
                panic!("inverse_cum_rate called on Library variant; use the per-person curve")
            }
        }
    }

    /// Duration of the infectious period. For `Constant` this is the
    /// mean of the `Exp` recovery distribution; for `Empirical` it's
    /// the curve's support (`points.last().0`); for `Library` it's the
    /// mean curve-support over the library (used only for display).
    #[allow(dead_code)]
    pub fn duration(&self) -> f64 {
        match self {
            InfectionRate::Constant { duration, .. } => *duration,
            InfectionRate::Empirical { points, .. } => empirical_curve_duration(points),
            InfectionRate::Library { rates, .. } => {
                if rates.is_empty() {
                    return 0.0;
                }
                let total: f64 = rates.iter().map(|r| empirical_curve_duration(r)).sum();
                total / rates.len() as f64
            }
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
            InfectionRate::Empirical { points, scale } => {
                validate_empirical_points(points)?;
                validate_scale(*scale)?;
            }
            InfectionRate::Library { rates, scale } => {
                if rates.is_empty() {
                    return Err(
                        "infection_rate library must contain at least one curve".to_string()
                    );
                }
                for (i, curve) in rates.iter().enumerate() {
                    validate_empirical_points(curve)
                        .map_err(|e| format!("infection_rate library curve #{i}: {e}"))?;
                }
                validate_scale(*scale)?;
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
        let r = InfectionRate::Empirical {
            points: vec![],
            scale: 1.0,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn empirical_must_be_sorted() {
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 0.5], [10.0, 0.2], [5.0, 0.1]],
            scale: 1.0,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn empirical_rejects_negative_first_time() {
        let r = InfectionRate::Empirical {
            points: vec![[-1.0, 0.5], [10.0, 0.5]],
            scale: 1.0,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn empirical_rejects_negative_rate() {
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 0.5], [10.0, -0.1]],
            scale: 1.0,
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
            scale: 1.0,
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
            scale: 1.0,
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
            scale: 1.0,
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
            scale: 1.0,
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
            scale: 1.0,
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
            scale: 1.0,
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
            scale: 1.0,
        };
        // Total area on [0, 5] = 0.5 · 1.0 · 5.0 = 2.5; anything beyond is unreachable.
        assert_eq!(trailing_zero.inverse_cum_rate(3.0), None);
        assert!(trailing_zero.inverse_cum_rate(1.0).is_some());
    }

    #[test]
    fn empirical_scale_multiplies_cum_rate() {
        // λ(τ) = 1 on [0, 10], scale = 0.5 → effective rate 0.5,
        // cum_rate(5) = 0.5 · 5 = 2.5 (raw cum is 5.0, halved).
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 1.0], [10.0, 1.0]],
            scale: 0.5,
        };
        assert!((r.cum_rate(5.0) - 2.5).abs() < 1e-12);
        // Inverse-CDF: target c = 2.5 ⇒ τ = 5.0 (since raw_cum(τ) =
        // c/scale = 5.0 ⇒ τ=5).
        let t = r.inverse_cum_rate(2.5).unwrap();
        assert!((t - 5.0).abs() < 1e-9);
    }

    #[test]
    fn empirical_scale_zero_makes_curve_unreachable() {
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 1.0], [10.0, 1.0]],
            scale: 0.0,
        };
        assert_eq!(r.cum_rate(5.0), 0.0);
        // Any positive target is unreachable when scale=0.
        assert_eq!(r.inverse_cum_rate(0.1), None);
    }

    #[test]
    fn empirical_rejects_negative_scale() {
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 0.5], [10.0, 0.5]],
            scale: -0.1,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn library_rejects_negative_scale() {
        let r = InfectionRate::Library {
            rates: vec![vec![[0.0, 0.5], [10.0, 0.5]]],
            scale: -1.0,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn empirical_serde_default_scale_is_one() {
        // Wire shape without `scale` should deserialize with scale=1.0.
        let json = r#"{"type":"empirical","points":[[0.0,0.5],[10.0,0.5]]}"#;
        let r: InfectionRate = serde_json::from_str(json).unwrap();
        match r {
            InfectionRate::Empirical { scale, .. } => assert_eq!(scale, 1.0),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn empirical_serde_roundtrip_json() {
        let r = InfectionRate::Empirical {
            points: vec![[0.0, 0.5], [20.0, 0.1]],
            scale: 1.0,
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

    // --- Curve (precomputed-cum) tests --------------------------------
    // The model's hot path runs cum_rate / inverse_cum_rate through
    // `Curve` rather than the slice-based helpers, so these lock in
    // bit-equivalence with the reference path plus the plateau/zero-
    // rate edge cases ixa-epi-isolation's EmpiricalRate covers.

    #[test]
    fn curve_new_builds_cumulative_trapezoid() {
        // cum[i] is the running trapezoid integral up to anchor i.
        // Tent shape: areas are 0.5, 1.5, 1.5, 0.5 → cum [0, 0.5, 2, 3.5, 4].
        let curve = Curve::new(vec![
            [0.0, 0.0],
            [1.0, 1.0],
            [2.0, 2.0],
            [3.0, 1.0],
            [4.0, 0.0],
        ]);
        assert_eq!(curve.cum, vec![0.0, 0.5, 2.0, 3.5, 4.0]);
    }

    #[test]
    fn curve_cum_rate_matches_slice_helper() {
        // Curve::cum_rate must agree exactly with the slice-based
        // helper at every point — both use the same trapezoid math.
        let points = vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [3.0, 1.0], [4.0, 0.0]];
        let curve = Curve::new(points.clone());
        for t in [-0.1, 0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.7, 4.0, 5.0] {
            assert_eq!(
                curve.cum_rate(t),
                empirical_cum_rate(&points, t),
                "mismatch at t={t}"
            );
        }
    }

    #[test]
    fn curve_inverse_cum_rate_matches_slice_helper() {
        let points = vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [3.0, 1.0], [4.0, 0.0]];
        let curve = Curve::new(points.clone());
        for c in [-0.1, 0.0, 0.1, 0.5, 1.125, 2.0, 2.875, 3.5, 4.0, 5.0] {
            assert_eq!(
                curve.inverse_cum_rate(c),
                empirical_inverse_cum_rate(&points, c),
                "mismatch at c={c}"
            );
        }
    }

    #[test]
    fn curve_inverse_cum_rate_walks_through_plateau() {
        // Mirrors ixa-epi-isolation's `test_inverse_cum_rate_plateaus`:
        // a zero-rate stretch in the middle keeps `cum` constant; an
        // inverse query exactly at the plateau value must return the
        // *earliest* time it's reached, and just above it must land
        // past the plateau on the next ramp.
        let curve = Curve::new(vec![
            [0.0, 1.0],
            [1.0, 1.0],
            [2.0, 0.0],
            [3.0, 0.0],
            [4.0, 1.0],
        ]);
        assert_eq!(curve.cum, vec![0.0, 1.0, 1.5, 1.5, 2.0]);
        let t_at_15 = curve.inverse_cum_rate(1.5).unwrap();
        assert!(
            (t_at_15 - 2.0).abs() < 1e-12,
            "expected t=2.0 at plateau start, got {t_at_15}"
        );
        let t_above = curve.inverse_cum_rate(1.6).unwrap();
        assert!(t_above > 3.0, "expected past plateau, got {t_above}");
    }

    #[test]
    fn curve_inverse_cum_rate_walks_through_leading_zero_segment() {
        // Curve starts with a zero-rate segment, then rate restarts.
        // Small c must land in the [1, 2] segment, not in the zero
        // prefix.
        let curve = Curve::new(vec![[0.0, 0.0], [1.0, 0.0], [2.0, 1.0]]);
        assert_eq!(curve.cum, vec![0.0, 0.0, 0.5]);
        let t = curve.inverse_cum_rate(0.01).unwrap();
        assert!(t > 1.0 && t < 2.0, "expected t in (1, 2), got {t}");
    }

    #[test]
    fn curve_cum_rate_zero_below_first_anchor() {
        // Anchor times need not start at 0; rate is 0 outside the
        // anchor range. (Same contract as ixa-epi-isolation's
        // `test_cum_rate_below_zero_rate`.)
        let curve = Curve::new(vec![[1.0, 1.0], [2.0, 3.0], [3.0, 5.0]]);
        assert_eq!(curve.cum_rate(0.0), 0.0);
        assert_eq!(curve.cum_rate(0.5), 0.0);
        assert_eq!(curve.cum, vec![0.0, 2.0, 6.0]);
    }

    #[test]
    fn curve_cum_rate_saturates_above_last_anchor() {
        // Past the curve's support, `cum_rate` equals the total
        // integral; `inverse_cum_rate` past that returns None.
        let curve = Curve::new(vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]]);
        let total = *curve.cum.last().unwrap();
        assert_eq!(total, 2.0);
        assert_eq!(curve.cum_rate(10.0), total);
        assert_eq!(curve.inverse_cum_rate(total + 0.1), None);
    }

    #[test]
    fn curve_duration_matches_last_anchor() {
        let curve = Curve::new(vec![[0.0, 0.5], [2.5, 0.5], [7.3, 0.2]]);
        assert_eq!(curve.duration(), 7.3);
        let empty = Curve::new(vec![]);
        assert_eq!(empty.duration(), 0.0);
    }
}
