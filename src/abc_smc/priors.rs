// Adapts `def_abc_smc/src/priors.rs`. Differences:
//   - r0 prior is Uniform (k88's choice) instead of def's Exp.
//   - `apply(...)` patches the existing `Parameters` rather than building
//     one from scratch, so the user-selected `infection_rate` shape +
//     `population` + `max_time` + `settings` carry through.
//   - R0 mapping onto `InfectionRate`:
//       * Constant  → set `value = r0 / duration`
//       * Empirical → set `scale = r0`
//       * Library   → set `scale = r0`
//     `model::run` then calls `normalize_to_r0`, so r0 reads as the
//     expected R0 under random mixing for every variant.

use rand::Rng;

use super::dist::{DiscreteUniform, Distribution, Normal, Uniform};
use super::stats::MeanVarianceEstimator;
use crate::parameters::Parameters;
use crate::rate::InfectionRate;

#[derive(Clone, Copy, Debug)]
pub struct CalibratedParams {
    pub r0: f64,
    pub initial_infections: u64,
    pub seed: u64,
}

pub struct Priors {
    pub initial_infections: DiscreteUniform<u64>,
    pub r0: Uniform,
}

impl Priors {
    pub fn new(
        initial_infections_lo: u64,
        initial_infections_hi: u64,
        r0_lo: f64,
        r0_hi: f64,
    ) -> Self {
        Self {
            initial_infections: DiscreteUniform::new(initial_infections_lo, initial_infections_hi),
            r0: Uniform::new(r0_lo, r0_hi),
        }
    }

    pub fn sample(&self, rng: &mut impl Rng, seed: u64) -> CalibratedParams {
        CalibratedParams {
            r0: self.r0.sample(rng),
            initial_infections: self.initial_infections.sample(rng),
            seed,
        }
    }

    pub fn weight(&self, value: &CalibratedParams) -> f64 {
        self.r0.weight(value.r0) * self.initial_infections.weight(value.initial_infections)
    }
}

/// Patch the base `Parameters` with the calibrated values for a single
/// model run.
pub fn apply(params: &CalibratedParams, base: &mut Parameters) {
    base.initial_infections = params.initial_infections as usize;
    base.seed = params.seed;
    base.infection_rate = match &base.infection_rate {
        InfectionRate::Constant { duration, .. } => InfectionRate::Constant {
            value: params.r0 / *duration,
            duration: *duration,
        },
        InfectionRate::Empirical { points, .. } => InfectionRate::Empirical {
            points: points.clone(),
            scale: params.r0,
        },
        InfectionRate::Library { rates, .. } => InfectionRate::Library {
            rates: rates.clone(),
            scale: params.r0,
        },
    };
}

pub struct PerturbationKernel {
    r0: Normal,
    initial_infections: DiscreteUniform<i64>,
    /// Probability a perturbed particle inherits its base particle's
    /// seed rather than drawing a fresh one. Matches cfa-calibration-
    /// tools `SeedKernel.prob_keep` (`perturbation_kernel.py:173`).
    /// `0.0` = always replace (max diversity); `1.0` = always inherit.
    prob_keep_seed: f64,
}

impl PerturbationKernel {
    /// r0 SD = `sqrt(variance_factor * sample_variance)`. Default 2.0 is
    /// Beaumont 2009 / cfa-calibration-tools (`variance_adapter.py:102`).
    /// We use Welford (divisor n-1); cfa's `np.var` uses n — < 1% SD
    /// difference at n ≥ 50.
    pub fn new<'a>(
        samples: impl Iterator<Item = &'a CalibratedParams>,
        variance_factor: f64,
        prob_keep_seed: f64,
    ) -> Self {
        let mut r0_stats = MeanVarianceEstimator::new();
        let mut n: u64 = 0;
        for x in samples {
            r0_stats.add(x.r0);
            n += 1;
        }
        // 1e-9 fallback: n < 2 → undefined variance; n ≥ 2 with all-equal
        // samples → SD = 0, which panics `Normal::new`.
        let sd = if n >= 2 {
            (variance_factor * r0_stats.get_variance()).sqrt().max(1e-9)
        } else {
            1e-9
        };
        PerturbationKernel {
            r0: Normal::new(0.0, sd),
            initial_infections: DiscreteUniform::new(0, 1),
            prob_keep_seed,
        }
    }

    pub fn perturb(&self, value: &CalibratedParams, rng: &mut impl Rng) -> CalibratedParams {
        CalibratedParams {
            r0: value.r0 + self.r0.sample(rng),
            initial_infections: (value.initial_infections as i64
                + self.initial_infections.sample(rng)) as u64,
            seed: if rng.random_bool(self.prob_keep_seed) {
                value.seed
            } else {
                rng.next_u64()
            },
        }
    }

    pub fn weight(&self, source: &CalibratedParams, value: &CalibratedParams) -> f64 {
        // Seed kernel transition probability: prob_keep_seed if the
        // seed was kept, (1 - prob_keep_seed) otherwise. Matches
        // cfa `SeedKernel.transition_probability` (perturbation_kernel.py:197).
        let seed_p = if source.seed == value.seed {
            self.prob_keep_seed
        } else {
            1.0 - self.prob_keep_seed
        };
        self.r0.weight(value.r0 - source.r0)
            * self
                .initial_infections
                .weight(value.initial_infections as i64 - source.initial_infections as i64)
            * seed_p
    }
}
