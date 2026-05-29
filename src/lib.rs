// Modules are `pub` so the `benches/` harness can construct
// `Parameters` and call `model::run` directly. The wasm-exposed surface
// is still just `simulate` / `simulate_batch` (the `#[wasm_bindgen]`
// items further down); nothing inside these modules is re-exported via
// wasm-bindgen.
pub mod abc_smc;
pub mod model;
pub mod normalize;
pub mod parameters;
pub mod person;
pub mod rate;
pub mod rate_library;
pub mod settings;
pub mod stats;

use cfasim_model::{model_outputs, ModelOutput};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::parameters::Parameters;
use crate::rate::InfectionRate;
use crate::settings::SettingType;

/// Arguments accepted by `simulate`. JS passes these as a single JSON string
/// (see `Page.vue`); `serde(rename_all = "camelCase")` bridges the JS field
/// names to the snake_case Rust ones.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SimulateArgs {
    infection_rate: InfectionRate,
    population: u32,
    initial_infections: u32,
    seed: u32,
    max_time: f64,
    n_simulations: u32,
    #[serde(default)]
    settings: Vec<SettingType>,
}

/// Arguments accepted by `simulate_batch`. Same as `SimulateArgs` but
/// expresses the ensemble as a sub-range: run `batch_size` simulations
/// with seeds `[seed + seed_offset, seed + seed_offset + batch_size)`.
/// Lets JS stream a large ensemble incrementally for progressive rendering.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SimulateBatchArgs {
    infection_rate: InfectionRate,
    population: u32,
    initial_infections: u32,
    seed: u32,
    max_time: f64,
    batch_size: u32,
    seed_offset: u32,
    #[serde(default)]
    settings: Vec<SettingType>,
}

struct BatchData {
    time: Vec<f64>,
    trajectories: Vec<Vec<f64>>,
    ar_per_run: Vec<f64>,
}

/// Run `batch_size` simulations with seeds offset by `seed_offset` from
/// `base.seed`. Returns the per-run trajectories and stats; aggregate
/// stats (median, summary) are the caller's responsibility.
fn run_batch(base: &Parameters, seed_offset: u32, batch_size: u32) -> BatchData {
    let mut time: Vec<f64> = Vec::new();
    let mut trajectories: Vec<Vec<f64>> = Vec::with_capacity(batch_size as usize);
    let mut ar_per_run: Vec<f64> = Vec::with_capacity(batch_size as usize);
    for i in 0..batch_size {
        let mut p = base.clone();
        p.seed = base.seed + seed_offset as u64 + i as u64;
        let s = model::run(p);
        let (t, values) = s.timeseries(base.max_time);
        if time.is_empty() {
            time = t;
        }
        ar_per_run.push(s.observed_attack_rate(base.population, base.initial_infections));
        trajectories.push(values);
    }
    BatchData {
        time,
        trajectories,
        ar_per_run,
    }
}

/// Stochastic SIR ensemble. Runs `n_simulations` independent realizations
/// (each with seed `seed + i`) and emits one `cumulative_infections_{i}`
/// column per run. The summary's observed attack rate is the median over
/// the ensemble.
#[wasm_bindgen]
pub fn simulate(args: &str) -> JsValue {
    let args: SimulateArgs = serde_json::from_str(args).expect("invalid simulate args");
    let base_params = Parameters {
        infection_rate: args.infection_rate,
        population: args.population as usize,
        initial_infections: args.initial_infections as usize,
        seed: args.seed as u64,
        max_time: args.max_time,
        settings: args.settings,
    };

    let BatchData {
        time,
        trajectories,
        ar_per_run,
    } = run_batch(&base_params, 0, args.n_simulations);

    let ar_median = stats::median(&ar_per_run);

    let n = time.len();
    let median = stats::pointwise_median(&trajectories);
    let mut series = ModelOutput::new(n).add_f64("time", time);
    for (i, traj) in trajectories.into_iter().enumerate() {
        series = series.add_f64(&format!("cumulative_infections_{i}"), traj);
    }
    series = series.add_f64("cumulative_infections_median", median);

    let summary = ModelOutput::new(1).add_f64("attack_rate_observed_median", vec![ar_median]);

    model_outputs([("series", series), ("summary", summary)])
}

/// Streaming-friendly variant of `simulate`. Runs a slice of the ensemble
/// (`batch_size` realizations starting at `seed + seed_offset`) and
/// returns the raw per-run data so JS can accumulate across batches and
/// re-render progressively. Aggregate statistics (median, observed summary)
/// are NOT computed here — the caller does that across all batches.
#[wasm_bindgen]
pub fn simulate_batch(args: &str) -> JsValue {
    let args: SimulateBatchArgs = serde_json::from_str(args).expect("invalid simulate_batch args");
    let base_params = Parameters {
        infection_rate: args.infection_rate,
        population: args.population as usize,
        initial_infections: args.initial_infections as usize,
        seed: args.seed as u64,
        max_time: args.max_time,
        settings: args.settings,
    };

    let BatchData {
        time,
        trajectories,
        ar_per_run,
    } = run_batch(&base_params, args.seed_offset, args.batch_size);

    let n = time.len();
    let mut series = ModelOutput::new(n).add_f64("time", time);
    for (i, traj) in trajectories.into_iter().enumerate() {
        series = series.add_f64(&format!("cumulative_infections_{i}"), traj);
    }

    let per_run =
        ModelOutput::new(args.batch_size as usize).add_f64("attack_rate_per_run", ar_per_run);

    model_outputs([("series", series), ("per_run", per_run)])
}

// -----------------------------------------------------------------------------
// ABC-SMC calibration — `calibrate_batch`
//
// Produces one batch of accepted particles for a given stage. JS owns the
// stage loop (computing thresholds from previous stage distances, persisting
// to IndexedDB between batches, etc). See `src/abc_smc/` for the algorithm
// port and CLAUDE.local.md for the bridge contract.
// -----------------------------------------------------------------------------

/// One particle from the previous stage, passed back into the next batch so
/// the wasm side can rebuild the perturbation kernel + WeightedIndex sampler
/// without keeping any persistent state. `weight` need not be normalized;
/// `WeightedIndex` treats it as proportional.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PriorParticle {
    r0: f64,
    initial_infections: u64,
    weight: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalibrateBatchArgs {
    // Model context (held fixed across the run).
    infection_rate: InfectionRate,
    population: u32,
    max_time: f64,
    #[serde(default)]
    settings: Vec<SettingType>,

    // Target data: per-integer-day observed series (dense, length
    // floor(max_time); value at day `d` is index `d - 1`). Expressed on
    // the scale selected by `cumulative` — daily or cumulative incidence.
    observed: Vec<u64>,
    /// 1-based days the target actually contains. The distance compares
    /// only at these days, skipping gaps (see `data_distance`). Empty =
    /// compare every day (dense fallback).
    #[serde(default)]
    observed_days: Vec<u64>,
    /// Scale the target is expressed on (and the model output is built to
    /// match before the gap-aware L1): `true` (default) = cumulative
    /// incidence, `false` = daily incidence. See `comparison_series`.
    #[serde(default = "default_cumulative")]
    cumulative: bool,

    // Uniform prior bounds.
    initial_infections_lo: u32,
    initial_infections_hi: u32,
    r0_lo: f64,
    r0_hi: f64,

    // Batch context.
    /// 0 = sample from prior; >0 = perturb from previous_particles.
    stage: u32,
    /// `None` (JS `null`) means accept everything (stage 0). JS sends
    /// `null` for INFINITY because `JSON.stringify(Infinity)` is `"null"`
    /// and f64 can't deserialize from a JSON null.
    error_threshold: Option<f64>,
    #[serde(default)]
    previous_particles: Vec<PriorParticle>,
    /// r0 kernel SD scaling — see `PerturbationKernel::new`. Default 2.0.
    #[serde(default = "default_variance_factor")]
    variance_factor: f64,
    /// Probability a perturbed particle inherits its base particle's
    /// seed. Default 0.0 = always replace. See `PerturbationKernel`.
    #[serde(default)]
    prob_keep_seed: f64,
    batch_size: u32,
    /// Just for ordering / output labels; does not affect the math.
    particle_offset: u32,
    seed: u64,
}

fn default_variance_factor() -> f64 {
    2.0
}

fn default_cumulative() -> bool {
    true
}

/// SplitMix64 finalizer — fast 64→64 hash with full avalanche. Used to
/// mix (seed, stage, offset) into a per-batch RNG seed without the
/// additive collision the older scheme had.
fn splitmix64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[wasm_bindgen]
pub fn calibrate_batch(args: &str) -> JsValue {
    use abc_smc::{perturb_from_previous_batch, sample_from_prior_batch, Particle, Priors};
    use rand::{rngs::StdRng, SeedableRng};

    let args: CalibrateBatchArgs =
        serde_json::from_str(args).expect("invalid calibrate_batch args");

    let base_params = Parameters {
        infection_rate: args.infection_rate,
        population: args.population as usize,
        initial_infections: 0, // overwritten per particle by `apply`
        seed: 0,               // overwritten per particle by `apply`
        max_time: args.max_time,
        settings: args.settings,
    };

    // Validate at the wasm boundary so panics from `model::run` (which
    // calls Parameters::validate.expect) can't reach here on
    // pathological priors. Errors propagate to JS as wasm traps and
    // surface in the runner's status line.
    assert!(
        args.r0_hi > args.r0_lo,
        "r0_hi ({}) must be > r0_lo ({})",
        args.r0_hi,
        args.r0_lo,
    );
    assert!(
        args.initial_infections_hi >= args.initial_infections_lo,
        "initial_infections_hi ({}) must be >= initial_infections_lo ({})",
        args.initial_infections_hi,
        args.initial_infections_lo,
    );
    assert!(
        args.initial_infections_hi as usize <= args.population as usize,
        "initial_infections_hi ({}) must be <= population ({})",
        args.initial_infections_hi,
        args.population,
    );
    // Series shape: `model::run` produces `floor(max_time)` per-day
    // values (daily or cumulative — same length), so observed must match.
    // Catches a too-short/too-long CSV upload before the distance runs.
    let expected_obs_len = args.max_time.floor().max(0.0) as usize;
    assert_eq!(
        args.observed.len(),
        expected_obs_len,
        "observed length ({}) must equal floor(max_time) ({})",
        args.observed.len(),
        expected_obs_len,
    );
    assert!(args.batch_size > 0, "batch_size must be >= 1");

    let priors = Priors::new(
        args.initial_infections_lo as u64,
        // DiscreteUniform's upper is exclusive (matches def). The UI
        // exposes an inclusive upper bound, so we add 1 at the wasm
        // boundary — kept here so the JS side sees an inclusive range.
        args.initial_infections_hi as u64 + 1,
        args.r0_lo,
        args.r0_hi,
    );

    // Deterministic batch seed via a 64-bit splitmix-style mix on the
    // (seed, stage, offset) triple — avoids the additive collision the
    // older `seed + stage*1_000_003 + offset` scheme could hit at large
    // particle counts (offset >= 1_000_003 wraps onto the next stage).
    let batch_seed = splitmix64(
        args.seed
            ^ splitmix64(args.stage as u64)
            ^ splitmix64(args.particle_offset as u64).rotate_left(17),
    );
    let mut rng = StdRng::seed_from_u64(batch_seed);

    let threshold = args.error_threshold.unwrap_or(f64::INFINITY);
    let step = if args.previous_particles.is_empty() {
        sample_from_prior_batch(
            threshold,
            args.batch_size as usize,
            &priors,
            &base_params,
            &args.observed,
            &args.observed_days,
            args.cumulative,
            &mut rng,
        )
    } else {
        // Rehydrate previous-stage particles. `model_output` isn't carried
        // across (we don't need it to build the kernel or compute weights).
        let prev: Vec<Particle> = args
            .previous_particles
            .iter()
            .map(|p| Particle {
                parameters: abc_smc::CalibratedParams {
                    r0: p.r0,
                    initial_infections: p.initial_infections,
                    seed: 0,
                },
                model_output: crate::stats::ModelStats::new(0),
                data_distance: 0,
                weight: p.weight,
            })
            .collect();
        perturb_from_previous_batch(
            threshold,
            args.batch_size as usize,
            &priors,
            &prev,
            args.variance_factor,
            args.prob_keep_seed,
            &base_params,
            &args.observed,
            &args.observed_days,
            args.cumulative,
            &mut rng,
        )
    };

    let n = step.particles.len();
    let mut r0_col = Vec::with_capacity(n);
    let mut ii_col = Vec::with_capacity(n);
    let mut weight_col = Vec::with_capacity(n);
    let mut dist_col = Vec::with_capacity(n);
    // Seeds are u64 but ModelOutput is f64-only; the upper 11 bits would
    // get rounded off if cast directly. Split into two u32 halves (each
    // fits exactly in f64) and let JS rejoin them into a string key.
    let mut seed_hi_col = Vec::with_capacity(n);
    let mut seed_lo_col = Vec::with_capacity(n);
    for p in &step.particles {
        r0_col.push(p.parameters.r0);
        ii_col.push(p.parameters.initial_infections as f64);
        weight_col.push(p.weight);
        dist_col.push(p.data_distance as f64);
        let s = p.parameters.seed;
        seed_hi_col.push((s >> 32) as f64);
        seed_lo_col.push((s & 0xffff_ffff) as f64);
    }

    let particles = ModelOutput::new(n)
        .add_f64("r0", r0_col)
        .add_f64("initial_infections", ii_col)
        .add_f64("weight", weight_col)
        .add_f64("distance", dist_col)
        .add_f64("seed_hi", seed_hi_col)
        .add_f64("seed_lo", seed_lo_col);

    // Per-particle daily-incidence trajectory, one column per particle.
    // Length = floor(max_time) (the cumulative timeseries diff). Powers
    // the per-stage trajectory overlay on the calibration page.
    let max_t = args.max_time.floor().max(0.0) as usize;
    let mut trajectories = ModelOutput::new(max_t);
    let day: Vec<f64> = (1..=max_t).map(|i| i as f64).collect();
    trajectories = trajectories.add_f64("day", day);
    for (i, p) in step.particles.iter().enumerate() {
        let (_, cum) = p.model_output.timeseries(args.max_time);
        let mut inc = Vec::with_capacity(max_t);
        for w in cum.windows(2) {
            inc.push((w[1] - w[0]).max(0.0));
        }
        // Pad if cum was shorter than expected (shouldn't happen, but
        // defensive — the ModelOutput requires all columns be same length).
        while inc.len() < max_t {
            inc.push(0.0);
        }
        inc.truncate(max_t);
        trajectories = trajectories.add_f64(&format!("traj_{i}"), inc);
    }

    let batch_meta = ModelOutput::new(1).add_f64("n_attempts", vec![step.n_attempts as f64]);

    model_outputs([
        ("particles", particles),
        ("trajectories", trajectories),
        ("batch_meta", batch_meta),
    ])
}
