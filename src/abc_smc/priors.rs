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
    seed_replace_prob: f64,
}

impl PerturbationKernel {
    /// Build the per-stage kernel from the previous generation's particle
    /// parameters. Mirrors `def_abc_smc/src/priors.rs::PerturbationKernel::new`
    /// (uses the marginal variance on r0; the discrete-uniform half-width
    /// on initial_infections is fixed to 1 to match def's TODO note).
    pub fn new<'a>(samples: impl Iterator<Item = &'a CalibratedParams>) -> Self {
        let mut r0_stats = MeanVarianceEstimator::new();
        let mut n: u64 = 0;
        for x in samples {
            r0_stats.add(x.r0);
            n += 1;
        }
        // Two clamps relative to def (documented divergence — see audit):
        //   1. n < 2: variance is undefined (divisor (n-1) is 0 / wraps).
        //      Fall back to SD = 1e-9.
        //   2. n >= 2 but all r0 samples identical → variance == 0 → SD == 0
        //      → `rand_distr::Normal::new(0, 0)` panics. Same fallback.
        // In both cases the kernel produces near-zero perturbations and
        // rejection sampling does the rest; this is a safety net, not the
        // expected path.
        let sd = if n >= 2 {
            r0_stats.get_variance().sqrt().max(1e-9)
        } else {
            1e-9
        };
        PerturbationKernel {
            r0: Normal::new(0.0, sd),
            initial_infections: DiscreteUniform::new(0, 1),
            seed_replace_prob: 1.0,
        }
    }

    pub fn perturb(&self, value: &CalibratedParams, rng: &mut impl Rng) -> CalibratedParams {
        // initial_infections perturbation in def uses DiscreteUniform(0, 1)
        // which only ever samples 0 (upper is exclusive). We keep that
        // behavior so the port stays faithful — initial_infections never
        // drifts from its base particle in v1. (Documented divergence
        // candidate; see audit.)
        CalibratedParams {
            r0: value.r0 + self.r0.sample(rng),
            initial_infections: (value.initial_infections as i64
                + self.initial_infections.sample(rng)) as u64,
            seed: if rng.random_bool(self.seed_replace_prob) {
                rng.next_u64()
            } else {
                value.seed
            },
        }
    }

    pub fn weight(&self, source: &CalibratedParams, value: &CalibratedParams) -> f64 {
        // Ignores the seed-replacement contribution, matching def.
        self.r0.weight(value.r0 - source.r0)
            * self
                .initial_infections
                .weight(value.initial_infections as i64 - source.initial_infections as i64)
    }
}
