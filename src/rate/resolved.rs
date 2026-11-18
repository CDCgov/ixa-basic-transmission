//! The infection rate as the model runs it. [`InfectionRate`] is the
//! configuration format; converting it validates it, lowers a parametric
//! rate to a curve, normalizes curves so `scale` is R₀ (see
//! [`normalize`](super::normalize)) and builds the rate functions, so the
//! model never sees an invalid or unlowered rate.

use super::normalize::normalize_curves;
use super::{AnyInfectionRate, EmpiricalRateFn, InfectionRate};

/// How a newly infected person gets their rate function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RateDraw {
    /// A constant rate over an infectious period drawn per person from an
    /// exponential with this mean.
    Constant { value: f64, mean_duration: f64 },
    /// A curve drawn uniformly from the library.
    Library,
}

pub struct ResolvedRate {
    /// The shared curves; empty for a constant rate.
    pub library: Vec<AnyInfectionRate>,
    pub draw: RateDraw,
}

impl TryFrom<&InfectionRate> for ResolvedRate {
    type Error = String;

    fn try_from(rate: &InfectionRate) -> Result<Self, String> {
        rate.validate()?;
        Ok(Self::build(rate, true))
    }
}

impl ResolvedRate {
    /// The rate with its curves as written, not normalized or validated: for
    /// tests of the loop against a curve with a known hazard.
    #[cfg(test)]
    pub(crate) fn as_written(rate: &InfectionRate) -> Self {
        Self::build(rate, false)
    }

    fn build(rate: &InfectionRate, normalize: bool) -> Self {
        let (mut curves, scale) = match rate {
            InfectionRate::Constant {
                value,
                exp_mean_duration,
            } => {
                return Self {
                    library: Vec::new(),
                    draw: RateDraw::Constant {
                        value: *value,
                        mean_duration: *exp_mean_duration,
                    },
                }
            }
            InfectionRate::Empirical { points, scale } => (vec![points.clone()], *scale),
            InfectionRate::Library { rates, scale } => (rates.clone(), *scale),
            InfectionRate::Parametric { dist, scale } => (vec![dist.curve()], *scale),
        };
        if normalize {
            normalize_curves(&mut curves);
        }
        // All curves share `scale`; per-curve heterogeneity is preserved by
        // the area normalization.
        Self {
            library: curves
                .into_iter()
                .map(|curve| EmpiricalRateFn::new(curve, scale).into())
                .collect(),
            draw: RateDraw::Library,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rate::{InvertibleRate, ParametricDist};

    fn total(rate_fn: &AnyInfectionRate) -> f64 {
        rate_fn.max_cum_rate()
    }

    #[test]
    fn constant_keeps_its_value_and_mean_duration() {
        let rate = InfectionRate::Constant {
            value: 0.5,
            exp_mean_duration: 3.0,
        };
        let resolved = ResolvedRate::try_from(&rate).unwrap();
        assert!(resolved.library.is_empty());
        assert_eq!(
            resolved.draw,
            RateDraw::Constant {
                value: 0.5,
                mean_duration: 3.0
            }
        );
    }

    #[test]
    fn curves_are_normalized_so_scale_is_r0() {
        let empirical = InfectionRate::Empirical {
            points: vec![[0.0, 4.0], [5.0, 4.0]],
            scale: 1.5,
        };
        let resolved = ResolvedRate::try_from(&empirical).unwrap();
        assert_eq!(resolved.draw, RateDraw::Library);
        assert_eq!(resolved.library.len(), 1);
        assert!((total(&resolved.library[0]) - 1.5).abs() < 1e-9);

        // A library is normalized by its mean area: areas 2 and 6 become
        // 0.5 and 1.5, times the scale.
        let library = InfectionRate::Library {
            rates: vec![vec![[0.0, 1.0], [2.0, 1.0]], vec![[0.0, 3.0], [2.0, 3.0]]],
            scale: 2.0,
        };
        let resolved = ResolvedRate::try_from(&library).unwrap();
        assert!((total(&resolved.library[0]) - 1.0).abs() < 1e-9);
        assert!((total(&resolved.library[1]) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn parametric_is_lowered_to_one_normalized_curve() {
        let rate = InfectionRate::Parametric {
            dist: ParametricDist::Gamma {
                shape: 2.0,
                scale: 1.5,
            },
            scale: 2.5,
        };
        let resolved = ResolvedRate::try_from(&rate).unwrap();
        assert_eq!(resolved.library.len(), 1);
        assert!((total(&resolved.library[0]) - 2.5).abs() < 1e-6);
    }

    #[test]
    fn an_invalid_rate_is_an_error() {
        let rate = InfectionRate::Empirical {
            points: vec![],
            scale: 1.0,
        };
        assert!(ResolvedRate::try_from(&rate).is_err());
    }

    #[test]
    fn as_written_leaves_curves_alone() {
        let rate = InfectionRate::Empirical {
            points: vec![[0.0, 4.0], [5.0, 4.0]],
            scale: 1.0,
        };
        let resolved = ResolvedRate::as_written(&rate);
        assert!((total(&resolved.library[0]) - 20.0).abs() < 1e-9);
    }
}
