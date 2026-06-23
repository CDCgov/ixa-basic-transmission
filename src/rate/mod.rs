//! Infectiousness rate functions.
//!
//! The [`InfectionRate`] enum is the **config / wire format** (serde-tagged,
//! consumed by the wasm args, calibration, presets, and the frontend). At model
//! entry it is resolved into [`RateFn`] values (a static-dispatch enum that
//! implements [`InfectiousnessRateFn`], held in [`storage::RateStorage`]) that
//! the transmission loop dispatches through. Concrete rate functions live in
//! the submodules: [`ConstantRate`], [`EmpiricalRate`], and the [`parametric`]
//! lowering.

use serde::{Deserialize, Serialize};

pub mod constant_rate;
pub mod empirical_rate;
pub mod normalize;
pub mod parametric;
pub mod rate_fn;
pub mod storage;

pub use constant_rate::ConstantRate;
pub use empirical_rate::{
    empirical_cum_rate, empirical_curve_duration, empirical_inverse_cum_rate,
    empirical_support_end, EmpiricalRate,
};
pub use normalize::normalize_to_r0;
pub use parametric::{materialize_parametric, ParametricDist, PARAMETRIC_GRID};
pub use rate_fn::{InfectiousnessRateFn, ScaledRateFn};
pub use storage::{build_empirical_library, AssignedRate, RateFn, RateStorage};

/// Per-person infectiousness rate, indexed by time since the person became
/// infectious (τ). Each variant carries its own duration since the two
/// are inseparable in the model: `Constant` needs an explicit duration
/// for `Exp` recovery, and `Empirical`'s recovery is derived from the
/// curve (when its integrated infectiousness is exhausted).
///
/// This enum is the configuration format only; the running model resolves it
/// into [`RateFn`] values via [`storage`].
///
/// - `Constant { value, duration }`: fixed rate, recovery is
///   `Exp(1/duration)`. `value` is an absolute hazard rate.
/// - `Empirical { points, scale }`: piecewise-linear infectiousness curve
///   in τ-space. Rate is 0 outside the anchor range. **The point values are
///   relative hazards** — `scale` converts them to absolute rates by
///   multiplication. Defaults to 1.0 so simple curves act as absolute rates
///   if the modeler hasn't calibrated.
/// - `Library { rates, scale }`: population of empirical curves with a
///   single shared scale. Each person gets one curve assigned at setup
///   and keeps it for their whole infectious period (per-person
///   heterogeneity). Empty curve list is rejected.
/// - `Parametric { dist, duration, scale }`: distribution-based shape,
///   **lowered to `Empirical`** at model entry (see [`materialize_parametric`]).
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
    /// Parametric time-varying shape: λ(τ) = `scale` · g(τ), where g is the
    /// density of `dist`. **Lowered to `Empirical` at model entry**
    /// (`materialize_parametric`) by sampling the kernel on a grid over the
    /// distribution's **auto-derived support** (no user-specified duration —
    /// the support is the distribution's own 99.9% mass point), so the whole
    /// pipeline downstream is identical to `Empirical`. `normalize_to_r0` then
    /// sets the curve area to 1, so `scale` is the expected R₀ under random
    /// mixing — same contract as `Empirical`.
    Parametric {
        dist: ParametricDist,
        #[serde(default = "default_scale")]
        scale: f64,
    },
}

fn default_scale() -> f64 {
    1.0
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
            InfectionRate::Parametric { dist, scale } => {
                dist.validate()?;
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
}
