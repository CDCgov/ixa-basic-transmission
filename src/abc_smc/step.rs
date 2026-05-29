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
//   - Distance is computed from `ModelStats`: `comparison_series` derives
//     a per-day series (daily incidence, or its running total when the
//     target is expressed as cumulative incidence) and `data_distance`
//     takes the gap-aware L1 against the observed target on the same scale.

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

/// Gap-aware L1 distance between observed and simulated series, scored
/// only at the days the target actually contains (`observed_days`,
/// 1-based) — gaps are skipped entirely, NOT scored as zero-vs-simulated
/// (the `cfa-calibration-tools` gap semantics; see
/// `cfa-evd-2025-interventions/data_processors.py::_estimate_symptom_onset_error`):
///
/// ```text
/// error = Σ_{d ∈ days} | obs[d] − sim[d] |
/// ```
///
/// The function is **representation-agnostic**: it's plain L1, so whether
/// it measures distance between daily-incidence or cumulative-incidence
/// curves depends purely on what the caller passes in. Both `observed`
/// and `simulated` must be on the SAME scale (the batch fns build the
/// simulated series to match `observed` via `comparison_series`).
///
/// `observed` / `simulated` are dense per-day series (value at day `d` is
/// index `d - 1`). A day present in `observed_days` but past the end of
/// either array contributes 0 there. If `observed_days` is empty every
/// day in `observed` is compared (dense fallback).
pub fn data_distance(observed: &[u64], simulated: &[u64], observed_days: &[u64]) -> u64 {
    let mut days: Vec<usize> = if observed_days.is_empty() {
        (1..=observed.len()).collect()
    } else {
        observed_days.iter().map(|&d| d as usize).collect()
    };
    // The caller may hand us arbitrary order (and duplicates after a hand
    // edit); dedup so a repeated day isn't double-counted.
    days.sort_unstable();
    days.dedup();

    let mut err: u128 = 0;
    for &d in &days {
        if d == 0 {
            continue; // 1-based; day 0 is out of range.
        }
        let idx = d - 1;
        let o = i128::from(observed.get(idx).copied().unwrap_or(0));
        let s = i128::from(simulated.get(idx).copied().unwrap_or(0));
        err += (o - s).unsigned_abs();
    }
    err as u64
}

/// Build the simulated series on the same scale as the observed target.
/// `cumulative = false` → daily incidence (the per-day diffs of the
/// model's cumulative counter). `cumulative = true` → cumulative
/// incidence (the running total of those daily values, so index `d - 1`
/// holds the case count through day `d`). The observed side is supplied
/// by the caller already in the matching representation.
fn comparison_series(stats: &ModelStats, max_time: f64, cumulative: bool) -> Vec<u64> {
    let daily = incidence_from_stats(stats, max_time);
    if !cumulative {
        return daily;
    }
    let mut running: u64 = 0;
    daily
        .into_iter()
        .map(|v| {
            running += v;
            running
        })
        .collect()
}

/// Mirrors `def_abc_smc/src/step.rs::initialize_abc_smc` (lines 82–123),
/// adapted to return a single batch instead of a full stage.
#[allow(clippy::too_many_arguments)]
pub fn sample_from_prior_batch(
    error_threshold: f64,
    batch_size: usize,
    priors: &Priors,
    base_params: &Parameters,
    observed: &[u64],
    observed_days: &[u64],
    cumulative: bool,
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
            let simulated = comparison_series(&model_output, base_params.max_time, cumulative);
            let dist = data_distance(observed, &simulated, observed_days);
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
    observed_days: &[u64],
    cumulative: bool,
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
            let simulated = comparison_series(&model_output, base_params.max_time, cumulative);
            let dist = data_distance(observed, &simulated, observed_days);
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
