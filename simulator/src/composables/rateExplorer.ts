// Pure helpers for the "Explore" tab: reduce any `InfectionRate` to a single
// effective λ(τ) curve (the rate the model actually samples, normalized so
// ∫λ = R₀ under random mixing), plus the cumulative Λ(τ) and its inverse for
// the inverse-CDF sampling demo.
//
// Reductions per variant:
//   - constant    → flat λ = value on [0, duration]
//   - empirical   → curve area-normalized to 1, ×scale
//   - parametric  → kernel sampled on a grid, area-normalized, ×scale
//   - library     → pointwise MEAN curve, area-normalized, ×scale
//                   (the model normalizes the library's mean area to 1, so
//                    the mean person's expected total is `scale`)

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

/** Pointwise samples used to approximate a Library's mean curve. */
const MEAN_SAMPLES = 101;

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

/** Pointwise mean of a library of curves, sampled on a 0 → maxSupport grid. */
function libraryMeanCurve(rates: [number, number][][]): {
  x: number[];
  y: number[];
} {
  if (!rates.length) return { x: [0, 1], y: [0, 0] };
  let tMax = 0;
  for (const c of rates) {
    const last = c[c.length - 1]?.[0] ?? 0;
    if (last > tMax) tMax = last;
  }
  if (tMax <= 0) return { x: [0, 1], y: [0, 0] };
  const x: number[] = [];
  const y: number[] = [];
  for (let i = 0; i < MEAN_SAMPLES; i++) {
    const t = (tMax * i) / (MEAN_SAMPLES - 1);
    x.push(t);
    let sum = 0;
    for (const c of rates) sum += rateAt(c, t);
    y.push(sum / rates.length);
  }
  return { x, y };
}

/** The effective λ(τ) curve the model samples for `rate`. */
export function effectiveRateCurve(rate: InfectionRate): RateCurve {
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
  if (rate.type === "empirical") {
    x = rate.points.map((p) => p[0]);
    rawY = rate.points.map((p) => p[1]);
  } else if (rate.type === "parametric") {
    const pts = parametricPoints(rate.dist, rate.duration);
    x = pts.map((p) => p[0]);
    rawY = pts.map((p) => p[1]);
  } else {
    const mean = libraryMeanCurve(rate.rates);
    x = mean.x;
    rawY = mean.y;
  }

  // Area-normalize the shape to 1, then scale to R₀ — mirrors the model's
  // `normalize_to_r0` so the demo's expected total matches the simulation.
  const a = area(x, rawY);
  const factor = a > 0 ? rate.scale / a : 0;
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
  // c(t) = R₀·CDF(t), the inverse is t = CDF⁻¹(c / R₀) — the quantile.
  let dHere: string;
  switch (rate.type) {
    case "constant":
      rHere = `a flat ${fmt(rate.value)} per day (a homogeneous Poisson process)`;
      cHere = `${fmt(rate.value)} · t (a straight line)`;
      dHere = `t = c / ${fmt(rate.value)} (Exponential inter-event times)`;
      break;
    case "parametric": {
      const d = rate.dist;
      if (d.dist === "weibull") {
        rHere = "R₀ × the Weibull PDF (probability density)";
        cHere = "R₀ × the Weibull CDF (cumulative distribution)";
        dHere = "t = scale · (−ln(1 − c / R₀))^(1 / shape)";
      } else if (d.dist === "lognormal") {
        rHere = "R₀ × the Lognormal PDF (probability density)";
        cHere = "R₀ × the Lognormal CDF (cumulative distribution)";
        dHere = "t = exp(μ + σ · Φ⁻¹(c / R₀)), Φ⁻¹ = standard-normal quantile";
      } else {
        rHere = "R₀ × the Gamma PDF (probability density)";
        cHere = "R₀ × the Gamma CDF (cumulative distribution)";
        dHere =
          "t = the Gamma quantile of c / R₀ (inverted numerically)";
      }
      break;
    }
    case "empirical":
      rHere = "R₀ × your anchor-point curve, area-normalized to 1";
      cHere = "R₀ × the area under that curve up to t, computed by trapezoidal integration";
      dHere =
        "t solves c(t) = c on the piecewise-linear rate (inverted numerically)";
      break;
    case "library":
      rHere = "R₀ × the population-mean curve, area-normalized to 1";
      cHere = "R₀ × the area under the mean curve up to t, computed by trapezoidal integration";
      dHere =
        "t solves c(t) = c on the mean curve (inverted numerically)";
      break;
  }
  return {
    r: `Expected infection attempts per day at time t since infection; the area under it is R₀. ${rHere}.`,
    c: `Expected attempts by time t given by ∫₀ᵗ r(s) ds = ${cHere}.`,
    d: `Given accumulated attempts c, it returns the time t. ${dHere}.`,
  };
}

/** Title + one-line description of `rate` for the explorer header. */
export function describeRate(rate: InfectionRate): {
  title: string;
  subtitle: string;
} {
  switch (rate.type) {
    case "constant":
      return {
        title: "Constant rate",
        subtitle: `${fmt(rate.value)} infection${
          rate.value === 1 ? "" : "s"
        } per day for ${fmt(rate.duration)} days`,
      };
    case "empirical":
      return {
        title: "Empirical curve",
        subtitle: `${rate.points.length} anchor points · R₀ = ${fmt(rate.scale)}`,
      };
    case "parametric": {
      const d = rate.dist;
      const name = d.dist.charAt(0).toUpperCase() + d.dist.slice(1);
      const params =
        d.dist === "lognormal"
          ? `μ=${fmt(d.mu)}, σ=${fmt(d.sigma)}`
          : `shape=${fmt(d.shape)}, scale=${fmt(d.scale)}`;
      return {
        title: `${name}(${params})`,
        subtitle: `R₀ = ${fmt(rate.scale)} over t ∈ [0, ${fmt(rate.duration)}]`,
      };
    }
    case "library":
      return {
        title: "Library (mean curve)",
        subtitle: `mean of ${rate.rates.length} per-person curves · R₀ = ${fmt(rate.scale)}`,
      };
  }
}
