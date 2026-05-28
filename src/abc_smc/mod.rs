// ABC-SMC ported from ~/gh/def/def_abc_smc/. Submodule layout mirrors
// def's so the algorithmic pieces can be cross-referenced file-by-file.
// JS owns the stage loop and persists particles per batch; this crate
// only knows how to produce one batch within a given stage. See
// CLAUDE.local.md for the wasm bridge shape.

pub mod dist;
pub mod particle;
pub mod priors;
pub mod stats;
pub mod step;

pub use particle::Particle;
pub use priors::{apply, CalibratedParams, PerturbationKernel, Priors};
pub use step::{
    data_distance, perturb_from_previous_batch, sample_from_prior_batch, ABCStepOutput,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parameters::Parameters;
    use crate::rate::InfectionRate;
    use rand::{rngs::StdRng, SeedableRng};

    fn truth_params() -> Parameters {
        Parameters {
            infection_rate: InfectionRate::Constant {
                value: 0.5, // r0 = value * duration = 0.5 * 3 = 1.5
                duration: 3.0,
            },
            population: 2000,
            initial_infections: 10,
            seed: 42,
            max_time: 60.0,
            settings: Vec::new(),
        }
    }

    fn synthetic_observed(p: &Parameters) -> Vec<u64> {
        let stats = crate::model::run(p.clone());
        let (_, cum) = stats.timeseries(p.max_time);
        let mut out = Vec::with_capacity(cum.len() - 1);
        for w in cum.windows(2) {
            out.push((w[1] - w[0]).max(0.0) as u64);
        }
        out
    }

    /// Smoke test: under a uniform prior that brackets the truth, the
    /// final-stage posterior should be biased toward the true r0. We use
    /// a small particle count + few stages for CI speed; the assertion is
    /// loose (posterior mean within ±0.6 of true r0 = 1.5).
    #[test]
    fn recovers_true_r0_on_synthetic_data() {
        let truth = truth_params();
        let observed = synthetic_observed(&truth);
        let true_r0 = 1.5;

        let mut base = truth.clone();
        base.initial_infections = 0;

        let priors = Priors::new(5, 21, 0.5, 3.0);
        let mut rng = StdRng::seed_from_u64(1);

        // Stage 0 with INF threshold.
        let gen0 =
            sample_from_prior_batch(f64::INFINITY, 60, &priors, &base, &observed, &[], &mut rng);
        assert_eq!(gen0.particles.len(), 60);

        // One refinement stage at the 30th-percentile distance.
        let mut dists: Vec<u64> = gen0.particles.iter().map(|p| p.data_distance).collect();
        dists.sort();
        let threshold = dists[(0.3 * dists.len() as f64) as usize - 1] as f64;

        let gen1 = perturb_from_previous_batch(
            threshold,
            60,
            &priors,
            &gen0.particles,
            2.0,
            0.0,
            &base,
            &observed,
            &[],
            &mut rng,
        );

        // Weighted mean r0 of the posterior.
        let total_w: f64 = gen1.particles.iter().map(|p| p.weight).sum();
        let weighted_r0: f64 = gen1
            .particles
            .iter()
            .map(|p| p.weight * p.parameters.r0)
            .sum::<f64>()
            / total_w;
        assert!(
            (weighted_r0 - true_r0).abs() < 0.6,
            "posterior r0 mean {weighted_r0} too far from true {true_r0}"
        );
    }

    /// Kernel perturbation variance ≈ `variance_factor * sample_variance`.
    /// Sample variance of [1..=5] is 2.5, so we check factor=1 → ~2.5 and
    /// factor=2 → ~5.0.
    #[test]
    fn perturbation_kernel_scales_with_variance_factor() {
        let samples = [1.0_f64, 2.0, 3.0, 4.0, 5.0];
        let params: Vec<CalibratedParams> = samples
            .iter()
            .map(|&r0| CalibratedParams {
                r0,
                initial_infections: 5,
                seed: 0,
            })
            .collect();
        let base = params[2];
        for (factor, lo, hi) in [(1.0_f64, 1.5, 3.5), (2.0, 3.5, 6.5)] {
            let kernel = PerturbationKernel::new(params.iter(), factor, 0.0);
            let mut rng = StdRng::seed_from_u64(99);
            let perturbed: Vec<f64> = (0..2000)
                .map(|_| kernel.perturb(&base, &mut rng).r0)
                .collect();
            let mean: f64 = perturbed.iter().sum::<f64>() / perturbed.len() as f64;
            let var: f64 = perturbed.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
                / (perturbed.len() - 1) as f64;
            assert!((mean - 3.0).abs() < 0.25, "factor={factor}: mean {mean}");
            assert!(
                (lo..hi).contains(&var),
                "factor={factor}: variance {var}, expected ~{}",
                factor * 2.5,
            );
        }
    }

    /// `weight()` includes the seed kernel transition probability:
    ///   prob_keep_seed     if source.seed == proposed.seed
    ///   1 - prob_keep_seed otherwise
    /// At prob_keep_seed = 0.0, mismatched seeds contribute 1.0 (no
    /// change vs. old behavior). Matched seeds contribute 0.0 — that
    /// branch is unreachable in normal operation but locked in here.
    #[test]
    fn perturbation_kernel_weight_includes_seed_transition() {
        let samples = [1.0_f64, 2.0, 3.0, 4.0, 5.0];
        let params: Vec<CalibratedParams> = samples
            .iter()
            .map(|&r0| CalibratedParams {
                r0,
                initial_infections: 5,
                seed: 0,
            })
            .collect();

        for keep in [0.0, 0.1, 0.5, 0.9] {
            let kernel = PerturbationKernel::new(params.iter(), 2.0, keep);
            let src = CalibratedParams {
                r0: 3.0,
                initial_infections: 5,
                seed: 42,
            };
            let same_seed = CalibratedParams { ..src };
            let diff_seed = CalibratedParams { seed: 123, ..src };
            // Same r0 + same initial_infections → r0 & ii contributions
            // are identical; the only difference is the seed factor.
            let w_same = kernel.weight(&src, &same_seed);
            let w_diff = kernel.weight(&src, &diff_seed);
            let ratio = w_same / w_diff.max(1e-300);
            let expected = keep / (1.0 - keep).max(1e-300);
            assert!(
                (ratio - expected).abs() < 1e-9,
                "keep={keep}: ratio {ratio} should equal keep/(1-keep) = {expected}"
            );
        }
    }

    #[test]
    fn data_distance_cumulative_dense() {
        // No gaps (observed_days empty → every day). Distance is the L1
        // distance between the two cumulative curves.
        assert_eq!(data_distance(&[1, 2, 3], &[1, 2, 3], &[]), 0);
        // obs cum = [0, 10]; sim cum = [5, 5]; |0-5| + |10-5| = 10.
        assert_eq!(data_distance(&[0, 10], &[5, 0], &[]), 10);
        assert_eq!(data_distance(&[], &[], &[]), 0);
    }

    #[test]
    fn data_distance_skips_gaps() {
        // Only days 1 and 3 are observed; day 2 is a gap and must NOT be
        // scored. obs[1]=5, obs[3]=4; sim[1]=5, sim[3]=1 (day-2 sim=99 is
        // ignored). cum at observed days: obs=[5,9], sim=[5,6];
        // |5-5| + |9-6| = 3.
        let observed = [5, 0, 4];
        let simulated = [5, 99, 1];
        assert_eq!(data_distance(&observed, &simulated, &[1, 3]), 3);
        // Order/duplicates in observed_days don't matter.
        assert_eq!(data_distance(&observed, &simulated, &[3, 1, 1]), 3);
    }

    #[test]
    fn data_distance_simulated_short_counts_as_zero() {
        // A day past the end of `simulated` contributes 0 on the sim side.
        // obs cum at days 1,2 = [2, 5]; sim has only day 1 (=2) so cum =
        // [2, 2]; |2-2| + |5-2| = 3.
        assert_eq!(data_distance(&[2, 3], &[2], &[1, 2]), 3);
    }

    #[test]
    fn apply_constant_maps_r0_to_value_over_duration() {
        let mut p = Parameters {
            infection_rate: InfectionRate::Constant {
                value: 0.0,
                duration: 4.0,
            },
            ..truth_params()
        };
        let cp = CalibratedParams {
            r0: 2.0,
            initial_infections: 3,
            seed: 7,
        };
        apply(&cp, &mut p);
        match p.infection_rate {
            InfectionRate::Constant { value, duration } => {
                assert!((value - 0.5).abs() < 1e-12);
                assert!((duration - 4.0).abs() < 1e-12);
            }
            _ => panic!("expected Constant"),
        }
        assert_eq!(p.initial_infections, 3);
        assert_eq!(p.seed, 7);
    }
}
