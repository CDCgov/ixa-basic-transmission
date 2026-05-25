mod model;
mod parameters;
mod stats;

use cfasim_model::{model_outputs, ModelOutput};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::parameters::Parameters;

/// Arguments accepted by `simulate`. JS passes these as a single JSON string
/// (see `Page.vue`); `serde(rename_all = "camelCase")` bridges the JS field
/// names to the snake_case Rust ones.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SimulateArgs {
    infection_rate: f64,
    infectious_period: f64,
    population: u32,
    initial_infections: u32,
    seed: u32,
    max_time: f64,
    n_simulations: u32,
}

/// Arguments accepted by `simulate_batch`. Same as `SimulateArgs` but
/// expresses the ensemble as a sub-range: run `batch_size` simulations
/// with seeds `[seed + seed_offset, seed + seed_offset + batch_size)`.
/// Lets JS stream a large ensemble incrementally for progressive rendering.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SimulateBatchArgs {
    infection_rate: f64,
    infectious_period: f64,
    population: u32,
    initial_infections: u32,
    seed: u32,
    max_time: f64,
    batch_size: u32,
    seed_offset: u32,
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
        infectious_period: args.infectious_period,
        population: args.population as usize,
        initial_infections: args.initial_infections as usize,
        seed: args.seed as u64,
        max_time: args.max_time,
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
        infectious_period: args.infectious_period,
        population: args.population as usize,
        initial_infections: args.initial_infections as usize,
        seed: args.seed as u64,
        max_time: args.max_time,
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
