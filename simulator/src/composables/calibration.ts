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
  priors: PriorBounds;
  // Relative-error schedule. stages[k] ∈ (0, 1) is the quantile of the
  // previous stage's sorted distances used as the threshold for stage k+1.
  // Total generations = 1 (prior sample at INF) + stages.length.
  stages: number[];
  nParticles: number;
  batchSize: number;
  seed: number;
  observed: number[];
  target: TargetSpec;
}

export type TargetSpec =
  | { mode: "synthetic"; truthInitialInfections: number; truthR0: number }
  | { mode: "csv"; filename: string };

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
    observed: [],
    target: { mode: "synthetic", truthInitialInfections: 10, truthR0: 1.5 },
  };
}

/// Mirrors `def_abc_smc/src/lib.rs` lines 49–51:
///   error_distance_index = (relative_error * sorted.len()) as usize - 1
///   error_threshold = sorted_distances[error_distance_index] as f64
/// Used by JS to compute the threshold for the next stage from the
/// previous stage's distances.
export function computeThreshold(
  prevParticles: Particle[],
  relativeError: number,
): number {
  if (prevParticles.length === 0) return Infinity;
  const sorted = prevParticles.map((p) => p.distance).sort((a, b) => a - b);
  const idx = Math.max(0, Math.floor(relativeError * sorted.length) - 1);
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
