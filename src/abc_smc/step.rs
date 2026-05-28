// Ports `def_abc_smc/src/step.rs`. Key differences:
//   - We replace def's `(n_particles, progress_callback)` with a
//     `batch_size` parameter and no callback. JS owns the stage loop and
//     persists each batch to IndexedDB. The wasm call is stateless across
//     batches; carry-over lives in `previous_particles`.
//   - Renormalization across the whole stage happens JS-side, NOT inside
//     each batch (def normalizes per-call because def's call IS the
//     stage). The raw weights returned here remain proportional to the
//     correct posterior weights, so `WeightedIndex` on the next stage
//     works whether the inputs are pre-normalized or not.
//   - Distance is computed from `ModelStats`: convert the cumulative
//     timeseries to daily incidence (matching def's
//     `symptomatic_incidence: Vec<u64>` shape) and sum |obs - sim|.

use rand::Rng;
use rand_distr::{weighted::WeightedIndex, Distribution as RandDistribution};

use super::particle::Particle;
use super::priors::{apply, PerturbationKernel, Priors};
use crate::model;
use crate::parameters::Parameters;
use crate::stats::ModelStats;

pub struct ABCStepOutput {
    pub particles: Vec<Particle>,
    pub n_attempts: u64,
}

/// Per-day incidence series derived from the model's cumulative timeseries.
/// Same shape def expects in `incidence_data`.
///
/// `ModelStats::timeseries` returns a single-run cumulative counter
/// (integer counts stored as f64; see `stats::record_infection`), so the
/// per-day differences are always non-negative integers exactly
/// representable in f64. `as u64` is exact here. If `timeseries` ever
/// starts returning interpolated/aggregated values, revisit.
fn incidence_from_stats(stats: &ModelStats, max_time: f64) -> Vec<u64> {
    let (_, cum) = stats.timeseries(max_time);
    if cum.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(cum.len() - 1);
    for w in cum.windows(2) {
        let inc = (w[1] - w[0]).max(0.0);
        out.push(inc as u64);
    }
    out
}

/// Sum of `|observed - simulated|` per day. Lengths must match — `zip`
/// would silently truncate to the shorter, which lets a too-short CSV
/// upload (or a too-long simulated trajectory) compare cleanly on the
/// overlap and ignore the tail, producing a spurious posterior. We
/// enforce equality and treat any padding the caller provides (zeros)
/// as part of the distance.
pub fn data_distance(observed: &[u64], simulated: &[u64]) -> u64 {
    assert_eq!(
        observed.len(),
        simulated.len(),
        "data_distance requires equal-length series (observed={}, simulated={})",
        observed.len(),
        simulated.len(),
    );
    observed
        .iter()
        .zip(simulated.iter())
        .map(|(x, y)| u64::abs_diff(*x, *y))
        .sum()
}

/// Mirrors `def_abc_smc/src/step.rs::initialize_abc_smc` (lines 82–123),
/// adapted to return a single batch instead of a full stage.
pub fn sample_from_prior_batch(
    error_threshold: f64,
    batch_size: usize,
    priors: &Priors,
    base_params: &Parameters,
    observed: &[u64],
    rng: &mut impl Rng,
) -> ABCStepOutput {
    let mut n_attempts: u64 = 0;
    let mut particles: Vec<Particle> = Vec::with_capacity(batch_size);
    for _ in 0..batch_size {
        loop {
            n_attempts += 1;
            let seed = rng.next_u64();
            let proposed = priors.sample(rng, seed);
            let mut p = base_params.clone();
            apply(&proposed, &mut p);
            let model_output = model::run(p);
            let simulated = incidence_from_stats(&model_output, base_params.max_time);
            let dist = data_distance(observed, &simulated);
            if dist as f64 <= error_threshold {
                particles.push(Particle {
                    parameters: proposed,
                    model_output,
                    data_distance: dist,
                    // Raw weight; renormalization happens JS-side once the
                    // full stage is assembled across batches.
                    weight: 1.0,
                });
                break;
            }
        }
    }
    ABCStepOutput {
        particles,
        n_attempts,
    }
}

/// Mirrors `def_abc_smc/src/step.rs::do_abc_smc_step` (lines 7–80).
/// `previous_particles` already carry their stage-normalized weights
/// (JS hands us either normalized or proportional weights — `WeightedIndex`
/// only cares about proportion). `variance_factor` and `prob_keep_seed`
/// are passed through to `PerturbationKernel::new`.
#[allow(clippy::too_many_arguments)]
pub fn perturb_from_previous_batch(
    error_threshold: f64,
    batch_size: usize,
    priors: &Priors,
    previous_particles: &[Particle],
    variance_factor: f64,
    prob_keep_seed: f64,
    base_params: &Parameters,
    observed: &[u64],
    rng: &mut impl Rng,
) -> ABCStepOutput {
    let weights: Vec<f64> = previous_particles.iter().map(|x| x.weight).collect();
    let particle_sampler = WeightedIndex::new(&weights).unwrap();
    let kernel = PerturbationKernel::new(
        previous_particles.iter().map(|x| &x.parameters),
        variance_factor,
        prob_keep_seed,
    );

    let mut n_attempts: u64 = 0;
    let mut particles: Vec<Particle> = Vec::with_capacity(batch_size);
    for _ in 0..batch_size {
        loop {
            n_attempts += 1;
            let base_particle = &previous_particles[particle_sampler.sample(rng)];
            let base_parameters = &base_particle.parameters;

            let mut proposed = kernel.perturb(base_parameters, rng);
            let mut proposed_prior_weight = priors.weight(&proposed);
            while proposed_prior_weight == 0.0 {
                proposed = kernel.perturb(base_parameters, rng);
                proposed_prior_weight = priors.weight(&proposed);
            }

            let mut p = base_params.clone();
            apply(&proposed, &mut p);
            let model_output = model::run(p);
            let simulated = incidence_from_stats(&model_output, base_params.max_time);
            let dist = data_distance(observed, &simulated);
            if dist as f64 <= error_threshold {
                // weight = prior(θ) / Σⱼ wⱼ K(θⱼ → θ). Matches def.
                let proposal_weight: f64 = previous_particles
                    .iter()
                    .map(|particle| {
                        particle.weight * kernel.weight(&particle.parameters, &proposed)
                    })
                    .sum();
                let weight = proposed_prior_weight / proposal_weight;
                particles.push(Particle {
                    parameters: proposed,
                    model_output,
                    data_distance: dist,
                    weight,
                });
                break;
            }
        }
    }
    ABCStepOutput {
        particles,
        n_attempts,
    }
}

/// Effective sample size. Mirrors `def_abc_smc/src/step.rs::get_ess`.
/// Expects normalized weights (Σ w = 1); the standard ESS formula.
#[allow(dead_code)]
pub fn ess(particles: &[Particle]) -> f64 {
    1.0 / particles.iter().map(|x| x.weight * x.weight).sum::<f64>()
}

/// Perplexity = exp(entropy). Mirrors `def_abc_smc/src/step.rs::get_perplexity`.
#[allow(dead_code)]
pub fn perplexity(particles: &[Particle]) -> f64 {
    let s: f64 = particles
        .iter()
        .map(|x| {
            if x.weight == 0.0 {
                0.0
            } else {
                -x.weight * x.weight.ln()
            }
        })
        .sum();
    s.exp()
}
