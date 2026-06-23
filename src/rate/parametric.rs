use serde::{Deserialize, Serialize};

use super::InfectionRate;

/// Number of grid anchors `materialize_parametric` samples the kernel at over
/// `[0, effective_support()]`. A smooth density is captured well at this
/// resolution; the per-event curve walk is ~3 ns regardless (see `benches/run.rs`).
pub const PARAMETRIC_GRID: usize = 128;

/// Fraction of the distribution's total mass the auto-truncation captures. The
/// support is cut at this quantile so the lowered curve is well-resolved; the
/// omitted tail is negligible and `normalize_to_r0` rescales the area to 1
/// regardless, so `scale = R₀` still holds exactly.
const SUPPORT_MASS_FRACTION: f64 = 0.999;

/// Trapezoid steps used to integrate the kernel when locating the truncation
/// quantile. Setup-time only (once per distinct curve), so generous is fine.
const SUPPORT_INTEGRATION_STEPS: usize = 8192;

/// A parametric infectiousness shape. Each variant's [`kernel`](Self::kernel)
/// is the distribution's density **up to a multiplicative constant** — the
/// model normalizes the curve area to 1 (`normalize_to_r0`), so the
/// normalizing constant (and any special functions, e.g. `Γ`, it would need)
/// cancels out and never has to be computed. Parameters use the standard
/// conventions noted per variant.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(
    tag = "dist",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum ParametricDist {
    /// Gamma with shape `k` and scale `θ`. Kernel `τ^(k−1)·e^(−τ/θ)`; mean `kθ`.
    Gamma { shape: f64, scale: f64 },
    /// Lognormal with log-space mean `mu` and SD `sigma` (`sigma > 0`).
    /// Kernel `(1/τ)·e^(−(ln τ − mu)² / (2·sigma²))`.
    Lognormal { mu: f64, sigma: f64 },
    /// Weibull with shape `k` and scale `λ`. Kernel `τ^(k−1)·e^(−(τ/λ)^k)`.
    Weibull { shape: f64, scale: f64 },
}

impl ParametricDist {
    /// Unnormalized density at `τ`. Zero for `τ ≤ 0` (and for any non-finite
    /// evaluation, e.g. the integrable singularity at 0 when `shape < 1`),
    /// which the area normalization then absorbs.
    #[inline]
    #[must_use]
    pub fn kernel(&self, t: f64) -> f64 {
        if t <= 0.0 {
            return 0.0;
        }
        let v = match *self {
            ParametricDist::Gamma { shape, scale } => t.powf(shape - 1.0) * (-t / scale).exp(),
            ParametricDist::Lognormal { mu, sigma } => {
                let z = (t.ln() - mu) / sigma;
                (1.0 / t) * (-0.5 * z * z).exp()
            }
            ParametricDist::Weibull { shape, scale } => {
                t.powf(shape - 1.0) * (-(t / scale).powf(shape)).exp()
            }
        };
        if v.is_finite() {
            v
        } else {
            0.0
        }
    }

    /// Sample the kernel on `PARAMETRIC_GRID` anchors over `[0, support]`.
    fn points(&self, support: f64) -> Vec<[f64; 2]> {
        let n = PARAMETRIC_GRID;
        (0..n)
            .map(|i| {
                let t = support * (i as f64) / ((n - 1) as f64);
                [t, self.kernel(t)]
            })
            .collect()
    }

    /// Auto-truncation point: the τ below which [`SUPPORT_MASS_FRACTION`] of the
    /// kernel's mass lies. The lowered empirical curve spans `[0, this]`, so
    /// there's no user-specified duration — the support is derived from the
    /// distribution itself (recovery then falls out as the max-cumulative reach
    /// time of the lowered curve, ≈ this point).
    fn effective_support(&self) -> f64 {
        let hi = self.tail_bound();
        let total = self.integrate_to(hi, SUPPORT_INTEGRATION_STEPS);
        if !total.is_finite() || total <= 0.0 {
            return hi;
        }
        let target = SUPPORT_MASS_FRACTION * total;
        let h = hi / SUPPORT_INTEGRATION_STEPS as f64;
        let mut cum = 0.0;
        let mut prev = self.kernel(0.0);
        for i in 1..=SUPPORT_INTEGRATION_STEPS {
            let t = h * i as f64;
            let v = self.kernel(t);
            cum += 0.5 * (prev + v) * h;
            if cum >= target {
                return t;
            }
            prev = v;
        }
        hi
    }

    /// An upper bound past which the kernel's tail is negligible. Scans outward
    /// geometrically, tracking the peak, until the density has decayed far below
    /// it (past the mode). Distribution-agnostic: the geometric growth spans many
    /// orders of magnitude, so it adapts to any scale.
    fn tail_bound(&self) -> f64 {
        const START: f64 = 1e-3;
        const GROWTH: f64 = 1.5;
        const REL: f64 = 1e-8;
        const MAX_STEPS: usize = 160;
        let mut peak = 0.0;
        let mut peak_t = 0.0;
        let mut t = START;
        for _ in 0..MAX_STEPS {
            let v = self.kernel(t);
            if v > peak {
                peak = v;
                peak_t = t;
            }
            if t > peak_t && peak > 0.0 && v < peak * REL {
                break;
            }
            t *= GROWTH;
        }
        t
    }

    /// Trapezoidal integral of the kernel over `[0, hi]` with `n` steps.
    fn integrate_to(&self, hi: f64, n: usize) -> f64 {
        let h = hi / n as f64;
        let mut cum = 0.0;
        let mut prev = self.kernel(0.0);
        for i in 1..=n {
            let v = self.kernel(h * i as f64);
            cum += 0.5 * (prev + v) * h;
            prev = v;
        }
        cum
    }

    pub fn validate(&self) -> Result<(), String> {
        let check = |name: &str, x: f64, strictly_positive: bool| {
            if !x.is_finite() || x < 0.0 || (strictly_positive && x <= 0.0) {
                Err(format!(
                    "parametric {name} must be a finite {}, got {x}",
                    if strictly_positive {
                        "positive number"
                    } else {
                        "non-negative number"
                    }
                ))
            } else {
                Ok(())
            }
        };
        match *self {
            ParametricDist::Gamma { shape, scale } | ParametricDist::Weibull { shape, scale } => {
                check("shape", shape, true)?;
                check("scale", scale, true)?;
            }
            ParametricDist::Lognormal { mu, sigma } => {
                if !mu.is_finite() {
                    return Err(format!("parametric mu must be finite, got {mu}"));
                }
                check("sigma", sigma, true)?;
            }
        }
        Ok(())
    }
}

/// Lower a `Parametric` rate to the equivalent `Empirical` curve by sampling
/// its kernel on a grid over the distribution's auto-derived support (see
/// [`ParametricDist::effective_support`]). No-op for every other variant. Call
/// once at model entry, **before** `normalize_to_r0` (which then makes the curve
/// area 1, so `scale = R₀`). Idempotent in effect — once lowered the rate is
/// `Empirical`.
pub fn materialize_parametric(rate: &mut InfectionRate) {
    if let InfectionRate::Parametric { dist, scale } = rate {
        let points = dist.points(dist.effective_support());
        let scale = *scale;
        *rate = InfectionRate::Empirical { points, scale };
    }
}

#[cfg(test)]
mod test {
    use super::{materialize_parametric, ParametricDist, PARAMETRIC_GRID};
    use crate::rate::InfectionRate;

    #[test]
    fn kernels_match_closed_forms() {
        // Gamma(shape=1): kernel = e^{-t/scale}. At t=2, scale=2 → e^{-1}.
        let g = ParametricDist::Gamma {
            shape: 1.0,
            scale: 2.0,
        };
        assert!((g.kernel(2.0) - (-1.0_f64).exp()).abs() < 1e-12);
        // Lognormal(mu=0, sigma=1): kernel = (1/t)·e^{-(ln t)²/2}. At t=1 → 1.
        let ln = ParametricDist::Lognormal {
            mu: 0.0,
            sigma: 1.0,
        };
        assert!((ln.kernel(1.0) - 1.0).abs() < 1e-12);
        // Weibull(shape=2, scale=1): kernel = t·e^{-t²}. At t=1 → e^{-1}.
        let w = ParametricDist::Weibull {
            shape: 2.0,
            scale: 1.0,
        };
        assert!((w.kernel(1.0) - (-1.0_f64).exp()).abs() < 1e-12);
        // Zero (and non-finite) for τ ≤ 0.
        assert_eq!(g.kernel(0.0), 0.0);
        assert_eq!(ln.kernel(-1.0), 0.0);
    }

    #[test]
    fn validate_rejects_bad_params() {
        assert!(ParametricDist::Gamma {
            shape: 0.0,
            scale: 1.0
        }
        .validate()
        .is_err());
        assert!(ParametricDist::Weibull {
            shape: 1.0,
            scale: -1.0
        }
        .validate()
        .is_err());
        assert!(ParametricDist::Lognormal {
            mu: 0.0,
            sigma: 0.0
        }
        .validate()
        .is_err());
        assert!(ParametricDist::Lognormal {
            mu: f64::NAN,
            sigma: 1.0
        }
        .validate()
        .is_err());
        // A Parametric with an invalid distribution is rejected at the
        // InfectionRate level (there is no duration field to validate).
        let bad_dist = InfectionRate::Parametric {
            dist: ParametricDist::Gamma {
                shape: 0.0,
                scale: 1.0,
            },
            scale: 1.0,
        };
        assert!(bad_dist.validate().is_err());
    }

    #[test]
    fn materialize_lowers_to_empirical_curve() {
        let mut r = InfectionRate::Parametric {
            dist: ParametricDist::Gamma {
                shape: 2.0,
                scale: 2.0,
            },
            scale: 2.5,
        };
        materialize_parametric(&mut r);
        match &r {
            InfectionRate::Empirical { points, scale } => {
                assert_eq!(points.len(), PARAMETRIC_GRID);
                assert_eq!(points[0][0], 0.0);
                // The support is auto-derived: for Gamma(2, 2) (mean 4) the 99.9%
                // mass point lands well past the mean. Just assert a sane,
                // finite, strictly-increasing grid.
                let last = points.last().unwrap()[0];
                assert!(last.is_finite() && last > 4.0, "support = {last}");
                // Scale is carried through untouched (normalize sets R₀ later).
                assert_eq!(*scale, 2.5);
                assert!(points.windows(2).all(|w| w[1][0] > w[0][0]));
            }
            _ => panic!("Parametric should lower to Empirical"),
        }
        // Idempotent: a second call is a no-op (already Empirical).
        let again = r.clone();
        materialize_parametric(&mut r);
        assert_eq!(r, again);
    }

    #[test]
    fn auto_truncation_captures_most_mass() {
        // The lowered curve should hold ≈ all the kernel's mass: its trapezoid
        // area over the auto-derived support should be within a fraction of a
        // percent of the kernel's full integral.
        let dist = ParametricDist::Weibull {
            shape: 2.0,
            scale: 5.0,
        };
        let mut r = InfectionRate::Parametric {
            dist: dist.clone(),
            scale: 1.0,
        };
        materialize_parametric(&mut r);
        let InfectionRate::Empirical { points, .. } = &r else {
            panic!("should lower to Empirical");
        };
        let captured: f64 = points
            .windows(2)
            .map(|w| 0.5 * (w[0][1] + w[1][1]) * (w[1][0] - w[0][0]))
            .sum();
        // Full integral to a far bound (Weibull(2, 5) is ~0 well before τ=60).
        let n = 200_000;
        let hi = 60.0;
        let h = hi / n as f64;
        let mut full = 0.0;
        let mut prev = dist.kernel(0.0);
        for i in 1..=n {
            let v = dist.kernel(h * i as f64);
            full += 0.5 * (prev + v) * h;
            prev = v;
        }
        assert!(captured / full > 0.99, "captured {captured} of {full}");
    }

    #[test]
    fn serde_roundtrip_json() {
        let r = InfectionRate::Parametric {
            dist: ParametricDist::Weibull {
                shape: 2.0,
                scale: 3.0,
            },
            scale: 1.0,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"type\":\"parametric\""));
        assert!(json.contains("\"dist\":\"weibull\""));
        let back: InfectionRate = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
