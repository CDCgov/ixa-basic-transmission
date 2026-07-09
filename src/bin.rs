//! Run the model from the command line
//!   cargo run -- <params.{toml,json}> [--sims N] [--seed S] [--out DIR]
//!
//! The file is a `config/parameters/*` preset (a `parameters` block, or a
//! bare params object) that deserializes straight into `Parameters`; a
//! `library` rate with empty `rates` is filled from the bundled curve CSV.
//!
//! Writes to a set of csv files.

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use ixa_basic_transmission::{model, parameters::Parameters, rate::InfectionRate, stats};

/// Run the transmission model on the host from a params file.
#[derive(Parser)]
#[command(about, long_about = None)]
struct Cli {
    /// Path to a params file (config-preset shape, .toml or .json).
    #[arg(default_value = "config/parameters/baseline.toml")]
    params: String,
    /// Number of simulations to run.
    #[arg(short = 'n', long, default_value_t = 1)]
    sims: u32,
    /// Base RNG seed (overrides `seed` in the file).
    #[arg(short, long)]
    seed: Option<u64>,
    /// Directory to write CSV output into (created if missing).
    #[arg(short, long, default_value = "output")]
    out: String,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();

    let raw =
        std::fs::read_to_string(&cli.params).map_err(|e| format!("reading {}: {e}", cli.params))?;
    let mut params = load_params(&cli.params, &raw)?;

    // Fill a `library` rate declared with empty `rates` from the bundled CSV,
    // same substitution the presets Vite plugin does.
    if let InfectionRate::Library { rates, .. } = &mut params.infection_rate {
        if rates.is_empty() {
            *rates = bundled_library();
        }
    }
    if let Some(seed) = cli.seed {
        params.seed = seed;
    }
    params.validate()?;

    let base_seed = params.seed;
    let n_sims = cli.sims.max(1);

    // One realization per seed `base_seed + i`
    let mut trajectories = Vec::with_capacity(n_sims as usize);
    let mut ars = Vec::with_capacity(n_sims as usize);
    for i in 0..n_sims {
        let mut p = params.clone();
        p.seed = base_seed + i as u64;
        let s = model::run(p);
        ars.push(s.observed_attack_rate(params.population, params.initial_infections));
        trajectories.push(s.timeseries(params.max_time).1);
    }
    let median_ar = stats::median(&ars);

    // Tidy long form, one (run, day, value) row each: incidence.csv holds the
    // daily new infections, cum_incidence.csv the running total they sum to.
    let mut incidence = String::from("run,day,incidence\n");
    let mut cum_incidence = String::from("run,day,cum_incidence\n");
    for (run, traj) in trajectories.iter().enumerate() {
        let mut prev = 0.0;
        for (day, &cum) in traj.iter().enumerate() {
            let _ = writeln!(incidence, "{run},{day},{}", cum - prev);
            let _ = writeln!(cum_incidence, "{run},{day},{cum}");
            prev = cum;
        }
    }

    let mut attack = String::from("run,attack_rate\n");
    for (run, ar) in ars.iter().enumerate() {
        let _ = writeln!(attack, "{run},{ar}");
    }

    let mut summary = String::from("metric,value\n");
    let _ = writeln!(summary, "n_simulations,{n_sims}");
    let _ = writeln!(summary, "population,{}", params.population);
    let _ = writeln!(summary, "initial_infections,{}", params.initial_infections);
    let _ = writeln!(summary, "base_seed,{base_seed}");
    let _ = writeln!(summary, "max_time,{}", params.max_time);
    let _ = writeln!(summary, "attack_rate_median,{median_ar}");

    let out = Path::new(&cli.out);
    std::fs::create_dir_all(out).map_err(|e| format!("creating {}: {e}", cli.out))?;
    for (name, body) in [
        ("incidence.csv", &incidence),
        ("cum_incidence.csv", &cum_incidence),
        ("attack_rate.csv", &attack),
        ("summary.csv", &summary),
    ] {
        std::fs::write(out.join(name), body).map_err(|e| format!("writing {name}: {e}"))?;
    }

    println!(
        "wrote {n_sims} sim(s) to {}/ (median attack rate {median_ar:.3})",
        cli.out
    );
    Ok(())
}

/// Deserialize the file's `parameters` block (or the whole doc if unwrapped)
/// into `Parameters`. Both formats go through `serde_json::Value` so the
/// path is format-agnostic. (`nSimulations` on the preset is ignored — the
/// run count comes from `--sims`.)
fn load_params(path: &str, raw: &str) -> Result<Parameters, String> {
    let doc: serde_json::Value = if path.ends_with(".toml") {
        toml::from_str(raw).map_err(|e| e.to_string())?
    } else {
        serde_json::from_str(raw).map_err(|e| e.to_string())?
    };
    let block = doc.get("parameters").cloned().unwrap_or(doc);
    serde_json::from_value(block).map_err(|e| e.to_string())
}

/// Parse the bundled long-format CSV (`id,time,value`) into one curve per `id`
/// (in file order), mirroring the frontend's `virtual:rateLibrary` plugin.
fn bundled_library() -> Vec<Vec<[f64; 2]>> {
    const CSV: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/config/rates/library_empirical_rate_fns.csv"
    ));
    let mut curves: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut last = "";
    for line in CSV.lines().skip(1).map(str::trim).filter(|l| !l.is_empty()) {
        let mut c = line.split(',');
        let id = c.next().unwrap_or("");
        let t = c.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let v = c.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        if id != last {
            curves.push(Vec::new());
            last = id;
        }
        curves.last_mut().unwrap().push([t, v]);
    }
    curves
}
