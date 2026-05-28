// Type definitions and pure helpers for the ABC-SMC calibration page.
// Algorithm lives in Rust (src/abc_smc/); this file is just the JS-side
// vocabulary + threshold computation + CSV parsing + histogram binning.

import type { InfectionRate } from "./infectionRate";
import type { SettingType } from "./settings";

/// Maps an R₀ value onto an `InfectionRate` shape, preserving everything
/// else. Mirrors `src/abc_smc/priors.rs::apply` so a JS-side caller
/// (synthetic-target generation, preview readouts) gets the same patch
/// the Rust calibrator applies per particle.
///   - `Constant` → `value = r0 / duration`
///   - `Empirical` → `scale = r0` (curve area is normalized to 1 by
///                                 `normalize_to_r0`)
///   - `Library`   → `scale = r0` (same normalization, applied to the
///                                 mean curve)
export function withR0(rate: InfectionRate, r0: number): InfectionRate {
  if (rate.type === "constant") {
    return { type: "constant", value: r0 / rate.duration, duration: rate.duration };
  }
  if (rate.type === "empirical") {
    return { ...rate, scale: r0 };
  }
  return { ...rate, scale: r0 };
}

/// Diff a cumulative-incidence series into daily-incidence values.
/// Mirrors `src/abc_smc/step.rs::incidence_from_stats` (clamp negatives
/// to 0, round to integer). Used by the synthetic-target generator on
/// the JS side and by the ABC distance metric on the Rust side; keeping
/// the two consistent matters because a CSV-vs-synthetic asymmetry here
/// would bias calibration.
export function cumulativeToIncidence(cumulative: ArrayLike<number>): number[] {
  const out: number[] = [];
  for (let i = 1; i < cumulative.length; i++) {
    out.push(Math.max(0, Math.round(cumulative[i] - cumulative[i - 1])));
  }
  return out;
}

// One particle stored after a wasm batch returns. Weights are the raw
// values out of wasm; renormalization to Σ w = 1 is computed on demand
// for display (histograms, posterior moments) and is not stored.
export interface Particle {
  r0: number;
  initialInfections: number;
  weight: number;
  distance: number;
  /// Per-particle simulated daily-incidence trajectory; length matches
  /// the target's `observed` array. Used for the per-stage overlay plot.
  trajectory: number[];
  /// The u64 RNG seed used to drive this particle's stochastic sim,
  /// serialized as `"<hi32>-<lo32>"` because JS numbers can't hold the
  /// full 64 bits losslessly. Powers the seed-observation diagnostic
  /// (`seedObservationHistogram`). Optional so runs persisted before
  /// SCHEMA_VERSION=2 still rehydrate without crashing.
  seed?: string;
}

export interface PriorBounds {
  r0Lo: number;
  r0Hi: number;
  initialInfectionsLo: number;
  initialInfectionsHi: number;
}

export interface ModelContext {
  infectionRate: InfectionRate;
  population: number;
  maxTime: number;
  settings: SettingType[];
}

export interface CalibrationConfig {
  modelContext: ModelContext;
  /// Initial infections for the synthetic-truth generator. Unused in
  /// CSV mode. The calibrated `initial_infections` is sampled from
  /// `priors.initialInfectionsLo..Hi` independently.
  initialInfections: number;
  priors: PriorBounds;
  // Quantile schedule, anchored to stage 0 (the prior-sample stage) — NOT
  // compounding stage-over-stage. stages[k] ∈ (0, 1) is the quantile of
  // stage 0's sorted distances used as the threshold for stage k+1.
  // So with stages = [0.5, 0.3, 0.2, 0.1], stage 1 keeps proposals within
  // the best 50% of prior samples, stage 4 within the best 10% — directly
  // comparable across runs because the reference distribution is fixed.
  // Total generations = 1 (prior sample at INF) + stages.length.
  stages: number[];
  nParticles: number;
  batchSize: number;
  seed: number;
  /// r0 perturbation kernel SD scaling — passed to wasm
  /// `PerturbationKernel::new`. Default 2.0 (Beaumont 2009);
  /// optional so older IDB rows rehydrate (treat undefined as 2.0).
  varianceFactor?: number;
  /// Probability an accepted particle inherits its base particle's
  /// seed. Default 0.0 = always replace; optional so older IDB rows
  /// rehydrate as 0.0.
  probKeepSeed?: number;
  /// Dense per-day observed incidence, length floor(maxTime). Value at
  /// day `d` lives at index `d - 1`; gap days are 0.
  observed: number[];
  /// 1-based days the target actually contains. The distance compares
  /// only at these days (cfa gap semantics — gaps are skipped, not scored
  /// as zero). Empty/undefined → every day (dense fallback, for rows
  /// persisted before this field existed).
  observedDays?: number[];
  /// Which incidence distance to calibrate against: `cumulative` (running
  /// totals, the cfa default) or `daily` (per-day values). Undefined →
  /// cumulative (older rows).
  distanceMetric?: "cumulative" | "daily";
  target: TargetSpec;
}

/// How the observed series was sourced. `synthetic` regenerates it from
/// the Model section's own infectionRate + initialInfections on each
/// Start; `csv` reads a user upload; `manual` starts blank and is built
/// by hand in the table editor. In every mode the values are editable in
/// the table — the mode only decides how the series is initially seeded
/// (and whether Start regenerates it, which only `synthetic` does).
export type TargetSpec =
  | { mode: "synthetic" }
  | { mode: "csv"; filename: string }
  | { mode: "manual" };

/// Default config for a fresh run. The model context defaults match
/// `Page.vue`'s `defaults` so users see familiar values.
export function defaultConfig(): CalibrationConfig {
  return {
    modelContext: {
      infectionRate: { type: "constant", value: 0.5, duration: 3.0 },
      population: 2000,
      maxTime: 60,
      settings: [],
    },
    initialInfections: 10,
    priors: {
      r0Lo: 0.5,
      r0Hi: 3.0,
      initialInfectionsLo: 1,
      initialInfectionsHi: 30,
    },
    stages: [0.5, 0.3, 0.2, 0.1],
    nParticles: 100,
    batchSize: 10,
    seed: 0,
    varianceFactor: 2.0,
    probKeepSeed: 0.0,
    observed: [],
    observedDays: [],
    distanceMetric: "cumulative",
    target: { mode: "manual" },
  };
}

/// Quantile lookup on a reference particle population's distances.
/// `fraction ∈ (0, 1)`: 0.5 → median, 0.1 → 10th percentile.
/// Called on stage 0's particles to derive each stage's threshold,
/// so thresholds stay anchored to the prior distribution instead of
/// compounding across stages.
export function computeThreshold(
  refParticles: Particle[],
  fraction: number,
): number {
  if (refParticles.length === 0) return Infinity;
  const sorted = refParticles.map((p) => p.distance).sort((a, b) => a - b);
  const idx = Math.max(0, Math.floor(fraction * sorted.length) - 1);
  return sorted[idx];
}

/// Parse a target-data CSV. Accepts either a header row (`time,incident_cases`
/// or any pair of column names) or no header. Returns the integer incidence
/// values in the order they appear; non-numeric or NaN cells throw.
export function parseTargetCsv(text: string): number[] {
  const lines = text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
  if (lines.length === 0) return [];
  // Skip header if the first row has non-numeric cells.
  const firstCells = lines[0].split(",");
  const hasHeader = firstCells.some(
    (c) => !Number.isFinite(Number(c.trim())),
  );
  const dataLines = hasHeader ? lines.slice(1) : lines;
  return dataLines.map((line, i) => {
    const cells = line.split(",");
    // Take the LAST column (the incidence). Common shapes: `time,inc`
    // or just `inc`.
    const raw = cells[cells.length - 1].trim();
    const v = Number(raw);
    if (!Number.isFinite(v) || v < 0) {
      throw new Error(`Invalid incidence at row ${i + (hasHeader ? 2 : 1)}: "${raw}"`);
    }
    return Math.round(v);
  });
}

export interface HistogramBin {
  /// Bin center for plotting.
  center: number;
  /// Sum of weights of particles falling in this bin.
  weight: number;
}

/// Weighted 1-D histogram. Returns evenly-spaced bins between `lo` and
/// `hi` (inclusive of `lo`, exclusive of `hi` except the last bin which
/// includes `hi`). Out-of-range particles are dropped. If all weights
/// are zero the bins are returned with zero weights.
export function weightedHistogram(
  values: number[],
  weights: number[],
  binCount: number,
  lo: number,
  hi: number,
): HistogramBin[] {
  if (binCount <= 0 || hi <= lo) return [];
  const width = (hi - lo) / binCount;
  const bins: HistogramBin[] = Array.from({ length: binCount }, (_, i) => ({
    center: lo + width * (i + 0.5),
    weight: 0,
  }));
  for (let i = 0; i < values.length; i++) {
    const v = values[i];
    if (!Number.isFinite(v)) continue;
    if (v < lo || v > hi) continue;
    let idx = Math.floor((v - lo) / width);
    if (idx >= binCount) idx = binCount - 1;
    bins[idx].weight += weights[i] ?? 0;
  }
  return bins;
}

/// Pair of parallel `x` and `y` arrays at evenly-spaced grid points,
/// ready to feed straight into a `LineChart` series.
export interface KdeResult {
  x: number[];
  y: number[];
}

/// Weighted Gaussian KDE on a regular grid spanning `[lo, hi]`.
///
/// Bandwidth is the **robust Silverman** plug-in `h = 0.9 · A · n_eff^(-1/5)`
/// with `A = min(σ_w, IQR_w / 1.34)`, chosen over the classic
/// `1.06·σ·n^(-1/5)` because ABC posteriors are routinely skewed and
/// the IQR-based scale won't over-smooth heavy tails. References:
/// Silverman 1986 §3.4.2 (the robust scale); Wand & Jones 1995. scipy's
/// `gaussian_kde(..., bw_method="silverman")` uses the non-robust form;
/// we deviate intentionally.
///
/// Weighted variance uses the bias correction `1/(1 − Σw²)` (matches
/// `np.cov(aweights=, bias=False)`). For ABC-SMC with `n_eff` in the
/// 30–60 range the uncorrected form underestimates σ² by ~3–5%.
///
/// `n_eff = (Σw)²/Σw² = 1/Σw²` for normalized weights (Kish 1965).
export function weightedKde(
  values: number[],
  weights: number[],
  lo: number,
  hi: number,
  nGrid: number,
): KdeResult {
  if (values.length === 0 || hi <= lo || nGrid < 2) {
    return { x: [], y: [] };
  }
  let mean = 0;
  for (let i = 0; i < values.length; i++) mean += weights[i] * values[i];
  let biasedVar = 0;
  for (let i = 0; i < values.length; i++) {
    const d = values[i] - mean;
    biasedVar += weights[i] * d * d;
  }
  let sumW2 = 0;
  for (const w of weights) sumW2 += w * w;
  const nEff = sumW2 > 0 ? 1 / sumW2 : values.length;
  // Bessel-analogue: divide by (1 − Σw²) for an unbiased σ² under
  // reliability weights (matches scipy/np.cov).
  const denom = 1 - sumW2;
  const variance = denom > 1e-12 ? biasedVar / denom : biasedVar;
  const sd = Math.sqrt(Math.max(variance, 1e-12));
  // Weighted IQR via cumulative weights walk after sorting.
  const idx = values.map((_, i) => i).sort((a, b) => values[a] - values[b]);
  const cum: number[] = new Array(idx.length);
  let acc = 0;
  for (let k = 0; k < idx.length; k++) {
    acc += weights[idx[k]];
    cum[k] = acc;
  }
  const totalW = cum.length > 0 ? cum[cum.length - 1] : 0;
  function quantile(q: number): number {
    if (idx.length === 0) return 0;
    const target = q * totalW;
    for (let k = 0; k < idx.length; k++) {
      if (cum[k] >= target) {
        if (k === 0) return values[idx[0]];
        const w0 = cum[k - 1];
        const w1 = cum[k];
        const v0 = values[idx[k - 1]];
        const v1 = values[idx[k]];
        const t = (target - w0) / Math.max(w1 - w0, 1e-12);
        return v0 + t * (v1 - v0);
      }
    }
    return values[idx[idx.length - 1]];
  }
  const iqr = quantile(0.75) - quantile(0.25);
  // Fall back to σ alone if IQR is degenerate (e.g. all particles tied
  // at the same value); avoids collapsing the bandwidth to 0.
  const a = iqr > 0 ? Math.min(sd, iqr / 1.34) : sd;
  const h = Math.max(0.9 * a * Math.pow(nEff, -1 / 5), 1e-9);
  const norm = 1 / (h * Math.sqrt(2 * Math.PI));
  const step = (hi - lo) / (nGrid - 1);
  const x: number[] = new Array(nGrid);
  const y: number[] = new Array(nGrid);
  for (let i = 0; i < nGrid; i++) {
    const xi = lo + i * step;
    let dens = 0;
    for (let j = 0; j < values.length; j++) {
      const z = (xi - values[j]) / h;
      dens += weights[j] * Math.exp(-0.5 * z * z);
    }
    x[i] = xi;
    y[i] = dens * norm;
  }
  return { x, y };
}

/// Sum of all particle weights — used to normalize displays on the fly.
export function totalWeight(particles: Particle[]): number {
  let s = 0;
  for (const p of particles) s += p.weight;
  return s;
}

/// Acceptance ratio = particles accepted / attempts. Per-stage.
export function acceptanceRatio(nAccepted: number, nAttempts: number): number {
  if (nAttempts === 0) return 0;
  return nAccepted / nAttempts;
}
