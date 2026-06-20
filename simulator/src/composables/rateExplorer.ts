// Pure helpers for the "Explore" tab: reduce any `InfectionRate` to a single
// effective λ(τ) curve (the rate the model actually samples, normalized so
// ∫λ = R₀ under random mixing), plus the cumulative Λ(τ) and its inverse for
// the inverse-CDF sampling demo.
//
// Reductions per variant:
//   - constant    → flat λ = value on [0, duration]
//   - empirical   → curve area-normalized to 1, ×scale
//   - parametric  → kernel sampled on a grid, area-normalized, ×scale
//   - library     → ONE assigned per-person curve (by index), scaled by
//                   `scale / mean-library-area` so it keeps its relative size.
//                   The model assigns each person a uniform-random curve at
//                   setup and normalizes the library's MEAN area to R₀, so an
//                   individual's expected total can sit above or below `scale`
//                   (per-curve heterogeneity preserved).

import type { InfectionRate } from "./infectionRate";
import { parametricPoints } from "./infectionRate";

export interface RateCurve {
  /** τ grid (ascending, starts at 0). */
  x: number[];
  /** Effective λ(τ) at each grid point; ∫λ over the support = `total`. */
  lambda: number[];
  /** Cumulative hazard Λ(τ_i) (trapezoid); `cum[0] = 0`. */
  cum: number[];
  /** Recovery time = end of support. */
  duration: number;
  /** ∫λ over [0, duration] = expected attempts = R₀ under random mixing. */
  total: number;
}

function cumulativeTrapezoid(x: number[], y: number[]): number[] {
  const cum = [0];
  for (let i = 1; i < x.length; i++) {
    cum.push(cum[i - 1] + 0.5 * (y[i - 1] + y[i]) * (x[i] - x[i - 1]));
  }
  return cum;
}

function area(x: number[], y: number[]): number {
  const cum = cumulativeTrapezoid(x, y);
  return cum[cum.length - 1] ?? 0;
}

/** Linear interpolation of a piecewise-linear curve at τ; 0 outside its range. */
function rateAt(curve: [number, number][], t: number): number {
  if (!curve.length) return 0;
  if (t < curve[0][0] || t > curve[curve.length - 1][0]) return 0;
  for (let i = 1; i < curve.length; i++) {
    const [t0, r0] = curve[i - 1];
    const [t1, r1] = curve[i];
    if (t <= t1) {
      const span = t1 - t0;
      if (span === 0) return r1;
      return r0 + ((t - t0) / span) * (r1 - r0);
    }
  }
  return 0;
}

/**
 * Mean curve-area across a library (= ∫ of the pointwise mean, by linearity).
 * The model normalizes this mean to R₀, leaving each person's curve at its own
 * relative size — so dividing a person's curve by the mean area (×scale)
 * preserves the per-curve heterogeneity.
 */
function libraryMeanArea(rates: [number, number][][]): number {
  if (!rates.length) return 0;
  let sum = 0;
  for (const c of rates) {
    sum += area(
      c.map((p) => p[0]),
      c.map((p) => p[1]),
    );
  }
  return sum / rates.length;
}

/**
 * The effective λ(τ) curve the model samples for `rate`. For a Library rate,
 * `libraryIndex` selects which per-person curve to show (the model assigns each
 * person a uniform-random curve at setup); ignored for the other variants.
 */
export function effectiveRateCurve(
  rate: InfectionRate,
  libraryIndex = 0,
): RateCurve {
  if (rate.type === "constant") {
    const x = [0, rate.duration];
    const lambda = [rate.value, rate.value];
    return {
      x,
      lambda,
      cum: cumulativeTrapezoid(x, lambda),
      duration: rate.duration,
      total: rate.value * rate.duration,
    };
  }

  let x: number[];
  let rawY: number[];
  // Area-normalize the shape, then scale to R₀ — mirrors the model's
  // `normalize_to_r0` so the demo's expected total matches the simulation.
  // Empirical/Parametric normalize their own area to 1; Library normalizes by
  // the *mean* library area so each assigned curve keeps its relative size.
  let factor: number;
  if (rate.type === "empirical") {
    x = rate.points.map((p) => p[0]);
    rawY = rate.points.map((p) => p[1]);
    const a = area(x, rawY);
    factor = a > 0 ? rate.scale / a : 0;
  } else if (rate.type === "parametric") {
    const pts = parametricPoints(rate.dist, rate.duration);
    x = pts.map((p) => p[0]);
    rawY = pts.map((p) => p[1]);
    const a = area(x, rawY);
    factor = a > 0 ? rate.scale / a : 0;
  } else {
    const n = rate.rates.length;
    const idx = n ? ((libraryIndex % n) + n) % n : 0;
    const picked: [number, number][] = n ? rate.rates[idx] : [[0, 0], [1, 0]];
    x = picked.map((p) => p[0]);
    rawY = picked.map((p) => p[1]);
    const meanArea = libraryMeanArea(rate.rates);
    factor = meanArea > 0 ? rate.scale / meanArea : 0;
  }

  const lambda = rawY.map((v) => v * factor);
  const cum = cumulativeTrapezoid(x, lambda);
  return {
    x,
    lambda,
    cum,
    duration: x[x.length - 1] ?? 0,
    total: cum[cum.length - 1] ?? 0,
  };
}

export interface LibraryOverlay {
  /** Each library curve, on the same normalized scale as `effectiveRateCurve`. */
  curves: { x: number[]; lambda: number[] }[];
  /** Pointwise mean of the normalized curves (red overlay); its area = R₀. */
  mean: { x: number[]; data: number[] };
}

const OVERLAY_MEAN_SAMPLES = 101;

/**
 * Every per-person curve in a Library, plus their pointwise mean — all scaled
 * by the same `scale / mean-library-area` factor `effectiveRateCurve` uses, so
 * the overlay sits on the same axes as the assigned-curve chart and the red
 * mean integrates to R₀. Empty for non-Library / empty rates.
 */
export function libraryOverlay(rate: InfectionRate): LibraryOverlay {
  if (rate.type !== "library" || !rate.rates.length) {
    return { curves: [], mean: { x: [], data: [] } };
  }
  const meanArea = libraryMeanArea(rate.rates);
  const factor = meanArea > 0 ? rate.scale / meanArea : 0;
  const curves = rate.rates.map((c) => ({
    x: c.map((p) => p[0]),
    lambda: c.map((p) => p[1] * factor),
  }));

  let tMax = 0;
  for (const c of rate.rates) {
    const last = c[c.length - 1]?.[0] ?? 0;
    if (last > tMax) tMax = last;
  }
  const x: number[] = [];
  const data: number[] = [];
  for (let i = 0; i < OVERLAY_MEAN_SAMPLES; i++) {
    const t = tMax > 0 ? (tMax * i) / (OVERLAY_MEAN_SAMPLES - 1) : 0;
    x.push(t);
    let sum = 0;
    for (const c of rate.rates) sum += rateAt(c, t);
    data.push((sum / rate.rates.length) * factor);
  }
  return { curves, mean: { x, data } };
}

/** Λ(τ) — cumulative hazard at time `t` (linear within a segment). */
export function cumulativeRate(curve: RateCurve, t: number): number {
  const { x, lambda, cum } = curve;
  if (t <= x[0]) return 0;
  for (let i = 0; i < x.length - 1; i++) {
    if (t < x[i + 1]) {
      const dt = t - x[i];
      const slope = (lambda[i + 1] - lambda[i]) / (x[i + 1] - x[i]);
      const rAtT = lambda[i] + slope * dt;
      return cum[i] + 0.5 * (lambda[i] + rAtT) * dt;
    }
  }
  return cum[cum.length - 1] ?? 0;
}

/**
 * Λ⁻¹(E): the time τ at which the cumulative hazard reaches `E` events.
 * `null` when `E` exceeds the curve's total (the person recovered before the
 * next event — the NHPP is exhausted). Solves the per-segment quadratic, the
 * same inversion the model uses.
 */
export function inverseCumulativeRate(
  curve: RateCurve,
  E: number,
): number | null {
  const { x, lambda, cum } = curve;
  if (E < 0) return null;
  if (E === 0) return 0;
  if (E > (cum[cum.length - 1] ?? 0) + 1e-12) return null;
  for (let i = 0; i < x.length - 1; i++) {
    if (E > cum[i + 1]) continue;
    const extra = E - cum[i];
    if (extra <= 0) return x[i];
    const span = x[i + 1] - x[i];
    const slope = (lambda[i + 1] - lambda[i]) / span;
    if (slope === 0) {
      if (lambda[i] === 0) continue; // zero-rate stretch: no events here
      return x[i] + extra / lambda[i];
    }
    const disc = Math.max(0, lambda[i] * lambda[i] + 2 * slope * extra);
    return x[i] + (-lambda[i] + Math.sqrt(disc)) / slope;
  }
  return null;
}

/**
 * Dense `(t, c(t))` samples for plotting the cumulative-rate curve. The true
 * c(t) is piecewise-quadratic (λ is piecewise-linear), but a chart draws
 * straight chords between points — on a coarse grid those chords can sit well
 * above the true curve (up to 2× near a segment midpoint where λ rises from
 * 0), so event markers at the exact `(τ, E)` appear off the line. Sampling
 * finely makes the rendered polyline track the true curve, so markers sit on
 * it.
 */
export function sampledCumulative(
  curve: RateCurve,
  n = 160,
): { x: number[]; data: number[] } {
  const d = curve.duration || 1;
  const x: number[] = [];
  const data: number[] = [];
  for (let i = 0; i < n; i++) {
    const t = (d * i) / (n - 1);
    x.push(t);
    data.push(cumulativeRate(curve, t));
  }
  return { x, data };
}

/** Exp(1) sample from a uniform draw `u ∈ [0, 1)`. Kept pure for testing. */
export function expFromUniform(u: number): number {
  return -Math.log(1 - u);
}

/**
 * One full inverse-CDF simulation of a single individual: draw Exp(1)
 * increments, accumulate them into the cumulative hazard, and invert through
 * Λ⁻¹ until a draw overshoots the curve's total area (the person recovers).
 * Returns the ascending list of infection event times τ — one realization of
 * the process (its length is the offspring count, mean ≈ R₀ = `curve.total`).
 * Same per-event accumulation as the interactive "Draw next" demo, run to
 * completion.
 *
 * `rng` returns a uniform in [0, 1) — pass `Math.random` in the app, a stub in
 * tests. The `MAX_EVENTS` ceiling only guards a degenerate rng; with real
 * draws the expected count is `curve.total`, far below it.
 */
export function simulateInfectionTimes(
  curve: RateCurve,
  rng: () => number,
): number[] {
  const taus: number[] = [];
  if (curve.total <= 0) return taus;
  const MAX_EVENTS = 100000;
  let cumulative = 0;
  while (taus.length < MAX_EVENTS) {
    cumulative += expFromUniform(rng());
    const tau = inverseCumulativeRate(curve, cumulative);
    if (tau === null) break;
    taus.push(tau);
  }
  return taus;
}

/** Offspring count from one run = number of event times before recovery. */
export function simulateInfectionCount(
  curve: RateCurve,
  rng: () => number,
): number {
  return simulateInfectionTimes(curve, rng).length;
}

/** `runs` independent runs, each returning that individual's event times. */
export function simulateInfectionRuns(
  curve: RateCurve,
  runs: number,
  rng: () => number = Math.random,
): number[][] {
  const out: number[][] = [];
  for (let i = 0; i < runs; i++) out.push(simulateInfectionTimes(curve, rng));
  return out;
}

/** Run `runs` independent `simulateInfectionCount`s, returning each count. */
export function simulateInfectionCounts(
  curve: RateCurve,
  runs: number,
  rng: () => number = Math.random,
): number[] {
  const out: number[] = [];
  for (let i = 0; i < runs; i++) out.push(simulateInfectionCount(curve, rng));
  return out;
}

export interface TimeHistogram {
  /** Bin labels (each bin's center τ, formatted) — parallel to `counts`. */
  categories: string[];
  /** Number of event times falling in each bin. */
  counts: number[];
  /** Per-bin share as a percentage of `total` (counts / total · 100). */
  percentages: number[];
  /** Center τ of each bin. */
  centers: number[];
  /** Bin width (= duration / bins). */
  binWidth: number;
  /** Total event times binned (those within [0, duration]). */
  total: number;
}

/**
 * Histogram of event times over `[0, duration]` in `bins` equal bins. Bars are
 * reported both as raw `counts` and as `percentages` of the total (the % view
 * stays bounded as samples accumulate). The shape approximates the (area = R₀)
 * rate curve r(t): the expected share in a bin is `∫bin r / R₀`, so pooling
 * many sampled times reconstructs r(t). Times outside `[0, duration]` are
 * dropped; the right edge falls in the last bin.
 */
export function eventTimeHistogram(
  times: number[],
  duration: number,
  bins = 20,
): TimeHistogram {
  const d = duration > 0 ? duration : 1;
  const n = Math.max(1, Math.floor(bins));
  const binWidth = d / n;
  const counts = new Array(n).fill(0);
  for (const t of times) {
    if (!Number.isFinite(t) || t < 0 || t > d) continue;
    let idx = Math.floor(t / binWidth);
    if (idx >= n) idx = n - 1;
    counts[idx]++;
  }
  const total = counts.reduce((a, b) => a + b, 0);
  const centers = counts.map((_, i) => (i + 0.5) * binWidth);
  const round1 = (v: number) => (Math.round(v * 10) / 10).toString();
  return {
    categories: centers.map(round1),
    counts,
    percentages: counts.map((c) => (total ? (c / total) * 100 : 0)),
    centers,
    binWidth,
    total,
  };
}

function fmt(v: number): string {
  if (!Number.isFinite(v)) return "—";
  return (Math.round(v * 100) / 100).toString();
}

/** Plain-language definitions of r/c/d, specialized to the selected rate. */
export interface RateFunctionDefs {
  r: string;
  c: string;
  d: string;
}

export function rateFunctionDefs(rate: InfectionRate): RateFunctionDefs {
  let rHere: string;
  let cHere: string;
  // The formula for the time t = d(c), given accumulated attempts c. Since
  // c(t) = R₀·CDF(t), the inverse is t = CDF⁻¹(c / R₀) — the quantile / inverse
  // CDF. For the parametric rates we keep the description deliberately vague
  // (just "the inverse CDF") rather than spelling out the closed-form quantile.
  let dHere: string;
  switch (rate.type) {
    case "constant":
      rHere = `${fmt(rate.value)}`;
      cHere = `${fmt(rate.value)} · t (a straight line)`;
      dHere = `t = c / ${fmt(rate.value)} (Exponential inter-event times)`;
      break;
    case "parametric": {
      const d = rate.dist;
      const name =
        d.dist === "weibull"
          ? "Weibull"
          : d.dist === "lognormal"
            ? "Lognormal"
            : "Gamma";
      rHere = `R₀ × the truncated ${name} PDF (renormalized over [0, duration])`;
      cHere = `R₀ × the truncated ${name} CDF (renormalized over [0, duration])`;
      dHere = `the inverse of that truncated, renormalized ${name} CDF (inverted numerically)`;
      break;
    }
    case "empirical":
      rHere = "R₀ × your anchor-point curve, area-normalized to 1";
      cHere = "R₀ × the area under that curve up to t, computed by trapezoidal integration";
      dHere =
        "t solves c(t) = c on the piecewise-linear rate (inverted numerically)";
      break;
    case "library":
      rHere =
        "the curve assigned to this individual from the library";
      cHere = "the area under the assigned curve up to t, computed by trapezoidal integration";
      dHere = "t solves c(t) = c on the assigned curve (inverted numerically)";
      break;
  }
  return {
    r: `Expected infections generated per day; the area under it is R₀. In this case, r(t) = ${rHere}.`,
    c: `Expected attempts by time t = ∫₀ᵗ r(s) ds, where r(s) is the rate function. In this case, c(t) = ${cHere}`,
    d: `Given accumulated attempts c, it returns the time t. For this rate function, ${dHere}.`,
  };
}

/** One-line description of `rate` for the explorer header. */
export function describeRate(rate: InfectionRate): { title: string } {
  switch (rate.type) {
    case "constant":
      return {
        title: `Constant rate of ${fmt(rate.value)} infection${
          rate.value === 1 ? "" : "s"
        } per day for ${fmt(rate.duration)} days`,
      };
    case "empirical":
      return {
        title: `Empirical curve of ${rate.points.length} anchor points · R₀ = ${fmt(rate.scale)}`,
      };
    case "parametric": {
      const d = rate.dist;
      const name = d.dist.charAt(0).toUpperCase() + d.dist.slice(1);
      const params =
        d.dist === "lognormal"
          ? `μ=${fmt(d.mu)}, σ=${fmt(d.sigma)}`
          : `shape=${fmt(d.shape)}, scale=${fmt(d.scale)}`;
      return {
        title: `${name} distributed ${params}, R₀ = ${fmt(rate.scale)} over t in [0, ${fmt(rate.duration)}]`,
      };
    }
    case "library":
      return {
        title: `Library of ${rate.rates.length} per-person curves · mean R₀ = ${fmt(rate.scale)}`,
      };
  }
}
