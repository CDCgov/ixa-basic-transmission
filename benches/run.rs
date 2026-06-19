//! Attributes the per-variant cost difference across three axes:
//!
//!   1. **end_to_end** — full `model::run`. Wall-clock for an identical
//!      workload across Constant / Empirical-5pt / Empirical-Npt / Library.
//!
//!   2. **setup_only** — `model::setup_only`: build Context, register
//!      Rate entities (Library only), create Person entities, assign
//!      AssignedRate (Library only). Does NOT execute. Isolates the
//!      per-person assignment + entity-creation overhead from the
//!      transmission loop.
//!
//!   3. **curve_eval** — pure hot-path math: a tight loop calling
//!      `empirical_cum_rate` + `empirical_inverse_cum_rate` on a curve
//!      of a given length. Isolates curve-walk cost per event.
//!
//! How to read the output:
//!
//!   - end_to_end(library) − end_to_end(empirical_long) ≈ Library
//!     dispatch overhead (the extra AssignedRate property lookup +
//!     RateLibraryPlugin lookup per event).
//!   - end_to_end(empirical_long) − end_to_end(empirical_short) ≈ cost
//!     of walking long curves on every event.
//!   - setup_only(library) − setup_only(constant) ≈ per-person
//!     assignment cost.
//!   - curve_eval(N=80) − curve_eval(N=5) ≈ per-event walk cost in
//!     isolation (no model overhead).

use std::fs;
use std::path::PathBuf;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use ixa_basic_transmission::model;
use ixa_basic_transmission::parameters::Parameters;
use ixa_basic_transmission::rate::{
    empirical_cum_rate, empirical_inverse_cum_rate, Curve, InfectionRate,
};

const POPULATION: usize = 2_000;
const INITIAL_INFECTIONS: usize = 5;
const MAX_TIME: f64 = 50.0;
const SEED: u64 = 7;

fn base_params(rate: InfectionRate) -> Parameters {
    Parameters {
        infection_rate: rate,
        population: POPULATION,
        initial_infections: INITIAL_INFECTIONS,
        seed: SEED,
        max_time: MAX_TIME,
        settings: Vec::new(),
    }
}

/// Loads `config/rates/library_empirical_rate_fns.csv` (relative to the
/// crate root). Rows are grouped by id and sorted by time. Panics on
/// missing file — benches should fail loudly.
fn load_bundled_library() -> Vec<Vec<[f64; 2]>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("config/rates/library_empirical_rate_fns.csv");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut groups: std::collections::BTreeMap<i64, Vec<[f64; 2]>> = Default::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.to_lowercase().starts_with("id,") {
            continue;
        }
        let mut it = line.split(',');
        let id: i64 = it.next().unwrap().parse().unwrap();
        let t: f64 = it.next().unwrap().parse().unwrap();
        let v: f64 = it.next().unwrap().parse().unwrap();
        groups.entry(id).or_default().push([t, v]);
    }
    groups
        .into_iter()
        .map(|(_, mut pts)| {
            pts.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());
            pts
        })
        .collect()
}

fn short_curve() -> Vec<[f64; 2]> {
    // Default 5-anchor viral-load curve used in the editor.
    vec![[0.0, 0.0], [2.0, 1.0], [4.0, 1.2], [6.0, 0.4], [8.0, 0.0]]
}

// -------------------- end-to-end (wall clock) --------------------

fn bench_end_to_end(c: &mut Criterion) {
    let library = load_bundled_library();
    let long_curve = library
        .iter()
        .max_by_key(|c| c.len())
        .expect("library has at least one curve")
        .clone();
    let short = short_curve();

    let avg_anchors = library.iter().map(|c| c.len()).sum::<usize>() as f64 / library.len() as f64;
    eprintln!(
        "library: {} curves, avg {:.1} anchors per curve, max {} anchors",
        library.len(),
        avg_anchors,
        long_curve.len()
    );

    let mut g = c.benchmark_group("end_to_end");
    g.sample_size(20);
    g.throughput(Throughput::Elements(POPULATION as u64));

    g.bench_function(BenchmarkId::from_parameter("constant"), |b| {
        let p = base_params(InfectionRate::Constant {
            value: 0.5,
            duration: 3.0,
        });
        b.iter(|| model::run(p.clone()));
    });

    g.bench_function(BenchmarkId::from_parameter("empirical_short_5pt"), |b| {
        let p = base_params(InfectionRate::Empirical {
            points: short.clone(),
            scale: 1.0,
        });
        b.iter(|| model::run(p.clone()));
    });

    g.bench_function(
        BenchmarkId::from_parameter(format!("empirical_long_{}pt", long_curve.len())),
        |b| {
            let p = base_params(InfectionRate::Empirical {
                points: long_curve.clone(),
                scale: 1.0,
            });
            b.iter(|| model::run(p.clone()));
        },
    );

    g.bench_function(BenchmarkId::from_parameter("library"), |b| {
        let p = base_params(InfectionRate::Library {
            rates: library.clone(),
            scale: 1.0,
        });
        b.iter(|| model::run(p.clone()));
    });

    g.finish();
}

// -------------------- setup only (isolates assignment cost) --------------------

fn bench_setup_only(c: &mut Criterion) {
    let library = load_bundled_library();
    let short = short_curve();

    let mut g = c.benchmark_group("setup_only");
    g.sample_size(30);

    g.bench_function(BenchmarkId::from_parameter("constant"), |b| {
        let p = base_params(InfectionRate::Constant {
            value: 0.5,
            duration: 3.0,
        });
        b.iter(|| model::setup_only(p.clone()));
    });

    g.bench_function(BenchmarkId::from_parameter("empirical_short_5pt"), |b| {
        let p = base_params(InfectionRate::Empirical {
            points: short.clone(),
            scale: 1.0,
        });
        b.iter(|| model::setup_only(p.clone()));
    });

    g.bench_function(BenchmarkId::from_parameter("library"), |b| {
        let p = base_params(InfectionRate::Library {
            rates: library.clone(),
            scale: 1.0,
        });
        b.iter(|| model::setup_only(p.clone()));
    });

    g.finish();
}

// -------------------- curve evaluation (per-event hot-path math) --------------------

fn bench_curve_eval(c: &mut Criterion) {
    let library = load_bundled_library();
    let long_points = library
        .iter()
        .max_by_key(|c| c.len())
        .expect("library has at least one curve")
        .clone();
    let short_points = short_curve();
    // Pre-build the Curves (with precomputed cum) once — same shape as
    // the model's hot path, which holds `&Curve` borrowed from a data
    // plugin populated at setup.
    let short = Curve::new(short_points.clone());
    let long_curve = Curve::new(long_points.clone());
    let long_len = long_curve.points.len();

    let mut g = c.benchmark_group("curve_eval");
    g.throughput(Throughput::Elements(1));

    // Simulates one transmission attempt's hot path: walk cum_rate at
    // `elapsed`, then inverse_cum_rate at `cum + e`.
    let elapsed = 1.5_f64;
    let e_sample = 0.3_f64;

    g.bench_function(BenchmarkId::from_parameter("short_5pt"), |b| {
        b.iter(|| {
            let cum = black_box(&short).cum_rate(black_box(elapsed));
            black_box(black_box(&short).inverse_cum_rate(black_box(cum + e_sample)));
        });
    });

    g.bench_function(
        BenchmarkId::from_parameter(format!("long_{long_len}pt")),
        |b| {
            b.iter(|| {
                let cum = black_box(&long_curve).cum_rate(black_box(elapsed));
                black_box(black_box(&long_curve).inverse_cum_rate(black_box(cum + e_sample)));
            });
        },
    );

    // Reference path (no precomputed cum) — same math the slice-based
    // helpers and `InfectionRate::cum_rate` use. Lets the diff measure
    // the precompute savings on its own.
    g.bench_function(BenchmarkId::from_parameter("short_5pt_no_cum"), |b| {
        b.iter(|| {
            let cum = empirical_cum_rate(black_box(&short_points), black_box(elapsed));
            black_box(empirical_inverse_cum_rate(
                black_box(&short_points),
                black_box(cum + e_sample),
            ));
        });
    });

    g.bench_function(
        BenchmarkId::from_parameter(format!("long_{long_len}pt_no_cum")),
        |b| {
            b.iter(|| {
                let cum = empirical_cum_rate(black_box(&long_points), black_box(elapsed));
                black_box(empirical_inverse_cum_rate(
                    black_box(&long_points),
                    black_box(cum + e_sample),
                ));
            });
        },
    );

    g.finish();
}

criterion_group!(
    benches,
    bench_end_to_end,
    bench_setup_only,
    bench_curve_eval
);
criterion_main!(benches);
