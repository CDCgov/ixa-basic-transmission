// Pure helpers for the "Modifiers" tab: two ways to simulate a modifier that
// switches on at t_m and scales r(t) by f from then on.
//   - Thinning (what the model does): forecast on the unmodified r(t), accept
//     with probability current / forecast. Needs f <= 1.
//   - Rescheduling: at t_m, cancel the pending forecast and redraw from t_m on
//     f * r(t).
// Both yield an NHPP of rate r(t) before t_m and f * r(t) after.

import type { RateCurve } from "./rateExplorer";
import {
  cumulativeRate,
  cumulativeTrapezoid,
  expFromUniform,
  inverseCumulativeRate,
  rateAtTime,
} from "./rateExplorer";

export interface ModifierSpec {
  /** Time (days since infection) the modifier switches on. */
  activatesAt: number;
  /** Multiplier on r(t) once active, >= 0. Above 1 rules thinning out. */
  factor: number;
}

/** Whether the modifier changes anything within the curve's support. */
export function modifierActive(curve: RateCurve, spec: ModifierSpec): boolean {
  return spec.factor !== 1 && spec.activatesAt < curve.duration;
}

/** Thinning needs r(t) to bound the modified rate, so f <= 1. */
export function thinningApplies(spec: ModifierSpec): boolean {
  return spec.factor <= 1;
}

/** The modified rate f·r(t) after activation, r(t) before. */
export function modifiedRateAt(
  curve: RateCurve,
  spec: ModifierSpec,
  t: number,
): number {
  const r = rateAtTime(curve, t);
  return t >= spec.activatesAt ? r * spec.factor : r;
}

/**
 * r(t) up to `activatesAt`, then f·r(t). The jump is two anchors at the same
 * τ; `cumulativeRate` / `inverseCumulativeRate` skip the zero-width segment.
 */
export function modifiedRateCurve(
  base: RateCurve,
  spec: ModifierSpec,
): RateCurve {
  if (!modifierActive(base, spec)) {
    return { ...base, x: [...base.x], lambda: [...base.lambda], cum: [...base.cum] };
  }
  const tm = spec.activatesAt;
  const x: number[] = [];
  const lambda: number[] = [];
  if (tm <= 0) {
    for (let i = 0; i < base.x.length; i++) {
      x.push(base.x[i]);
      lambda.push(base.lambda[i] * spec.factor);
    }
  } else {
    const rTm = rateAtTime(base, tm);
    for (let i = 0; i < base.x.length; i++) {
      if (base.x[i] < tm) {
        x.push(base.x[i]);
        lambda.push(base.lambda[i]);
      }
    }
    x.push(tm, tm);
    lambda.push(rTm, rTm * spec.factor);
    for (let i = 0; i < base.x.length; i++) {
      if (base.x[i] > tm) {
        x.push(base.x[i]);
        lambda.push(base.lambda[i] * spec.factor);
      }
    }
  }
  const cum = cumulativeTrapezoid(x, lambda);
  return {
    x,
    lambda,
    cum,
    duration: base.duration,
    total: cum[cum.length - 1] ?? 0,
  };
}

const MAX_ATTEMPTS = 100000;

// --- Thinning ---------------------------------------------------------------

export interface ThinningDraw {
  e: number;
  u: number;
}

export interface ThinningAttempt {
  e: number;
  /** Running Σe on the unmodified curve. */
  cumulative: number;
  /** Forecast time on the unmodified curve; `null` once exhausted. */
  tau: number | null;
  /** current ÷ forecast: 1 before activation, f after. */
  pAccept: number;
  /** Uniform compared against `pAccept`; `null` when none was needed. */
  u: number | null;
  outcome: "accepted" | "rejected" | "recovered";
}

/**
 * One thinning attempt from running total `cumulative`. The acceptance test
 * reads the modifier state at the forecast time, as `evaluate_forecast` does;
 * `drawU` is only called when a test is needed.
 */
function thinningStep(
  curve: RateCurve,
  spec: ModifierSpec,
  cumulative: number,
  e: number,
  drawU: () => number,
): ThinningAttempt {
  const c = cumulative + e;
  const tau = inverseCumulativeRate(curve, c);
  if (tau === null) {
    return { e, cumulative: c, tau, pAccept: 1, u: null, outcome: "recovered" };
  }
  const pAccept = tau >= spec.activatesAt ? spec.factor : 1;
  if (pAccept >= 1) {
    return { e, cumulative: c, tau, pAccept: 1, u: null, outcome: "accepted" };
  }
  const u = drawU();
  const outcome = u < pAccept ? "accepted" : "rejected";
  return { e, cumulative: c, tau, pAccept, u, outcome };
}

/** Replay a thinning walk from its raw draws; stops at recovery. */
export function thinningAttempts(
  curve: RateCurve,
  spec: ModifierSpec,
  draws: readonly ThinningDraw[],
): ThinningAttempt[] {
  const out: ThinningAttempt[] = [];
  if (!thinningApplies(spec)) return out;
  let cumulative = 0;
  for (const { e, u } of draws) {
    const a = thinningStep(curve, spec, cumulative, e, () => u);
    out.push(a);
    if (a.outcome === "recovered") break;
    cumulative = a.cumulative;
  }
  return out;
}

export interface ThinningRun {
  accepted: number[];
  rejected: number[];
}

/** One individual to recovery under thinning. `rng` returns U[0, 1). */
export function simulateThinning(
  curve: RateCurve,
  spec: ModifierSpec,
  rng: () => number,
): ThinningRun {
  const accepted: number[] = [];
  const rejected: number[] = [];
  if (curve.total <= 0 || !thinningApplies(spec)) return { accepted, rejected };
  let cumulative = 0;
  for (let n = 0; n < MAX_ATTEMPTS; n++) {
    const a = thinningStep(curve, spec, cumulative, expFromUniform(rng()), rng);
    if (a.outcome === "recovered") break;
    (a.outcome === "accepted" ? accepted : rejected).push(a.tau as number);
    cumulative = a.cumulative;
  }
  return { accepted, rejected };
}

export function simulateThinningRuns(
  curve: RateCurve,
  spec: ModifierSpec,
  runs: number,
  rng: () => number = Math.random,
): ThinningRun[] {
  const out: ThinningRun[] = [];
  for (let i = 0; i < runs; i++) out.push(simulateThinning(curve, spec, rng));
  return out;
}

// --- Rescheduling -----------------------------------------------------------

export interface RescheduleAttempt {
  e: number;
  /** Inverted on the modified curve f·r(t) rather than r(t). */
  onModified: boolean;
  /** Start of the forecast: the previous event, or t_m after a cancellation. */
  from: number;
  /** Forecast time on the curve used; `null` once exhausted. */
  tau: number | null;
  /**
   * `cancelled`: a pending r(t) forecast reached past t_m, so activation
   * discards it. `none`: r(t) was exhausted before t_m, so nothing was pending
   * (the model schedules no plan); activation still starts the walk at t_m.
   */
  outcome: "event" | "cancelled" | "none" | "recovered";
  /** This is the draw made at t_m; always follows a `cancelled` / `none` row. */
  rescheduled: boolean;
}

interface RescheduleState {
  onModified: boolean;
  from: number;
  rescheduled: boolean;
}

function initialRescheduleState(curve: RateCurve, spec: ModifierSpec): RescheduleState {
  return {
    onModified: modifierActive(curve, spec) && spec.activatesAt <= 0,
    from: 0,
    rescheduled: false,
  };
}

/** One rescheduling forecast from `state`, plus the state for the next one. */
function reschedulingStep(
  curve: RateCurve,
  modified: RateCurve,
  spec: ModifierSpec,
  state: RescheduleState,
  e: number,
): { attempt: RescheduleAttempt; next: RescheduleState } {
  const used = state.onModified ? modified : curve;
  const tau = inverseCumulativeRate(used, cumulativeRate(used, state.from) + e);
  const base = { e, onModified: state.onModified, from: state.from, tau, rescheduled: state.rescheduled };
  const crossesActivation = tau === null || tau >= spec.activatesAt;
  if (modifierActive(curve, spec) && !state.onModified && crossesActivation) {
    return {
      attempt: { ...base, outcome: tau === null ? "none" : "cancelled" },
      next: { onModified: true, from: spec.activatesAt, rescheduled: true },
    };
  }
  if (tau === null) return { attempt: { ...base, outcome: "recovered" }, next: state };
  return {
    attempt: { ...base, outcome: "event" },
    next: { onModified: state.onModified, from: tau, rescheduled: false },
  };
}

/** Replay a rescheduling walk from its raw Exp(1) draws; stops at recovery. */
export function reschedulingAttempts(
  curve: RateCurve,
  modified: RateCurve,
  spec: ModifierSpec,
  draws: readonly number[],
): RescheduleAttempt[] {
  const out: RescheduleAttempt[] = [];
  let state = initialRescheduleState(curve, spec);
  for (const e of draws) {
    const { attempt, next } = reschedulingStep(curve, modified, spec, state, e);
    out.push(attempt);
    if (attempt.outcome === "recovered") break;
    state = next;
  }
  return out;
}

/** The walk is waiting on the draw that activation triggers at t_m. */
export function needsRedraw(attempts: readonly RescheduleAttempt[]): boolean {
  const last = attempts[attempts.length - 1]?.outcome;
  return last === "cancelled" || last === "none";
}

export interface RescheduleRun {
  events: number[];
  /** Forecasts thrown away at activation: 0 or 1 per individual. */
  cancelled: number;
}

/** One individual to recovery under rescheduling. `rng` returns U[0, 1). */
export function simulateRescheduling(
  curve: RateCurve,
  modified: RateCurve,
  spec: ModifierSpec,
  rng: () => number,
): RescheduleRun {
  const events: number[] = [];
  let cancelled = 0;
  if (curve.total <= 0) return { events, cancelled };
  let state = initialRescheduleState(curve, spec);
  for (let n = 0; n < MAX_ATTEMPTS; n++) {
    const e = expFromUniform(rng());
    const { attempt, next } = reschedulingStep(curve, modified, spec, state, e);
    if (attempt.outcome === "recovered") break;
    if (attempt.outcome === "event") events.push(attempt.tau as number);
    if (attempt.outcome === "cancelled") cancelled++;
    state = next;
  }
  return { events, cancelled };
}

export function simulateReschedulingRuns(
  curve: RateCurve,
  modified: RateCurve,
  spec: ModifierSpec,
  runs: number,
  rng: () => number = Math.random,
): RescheduleRun[] {
  const out: RescheduleRun[] = [];
  for (let i = 0; i < runs; i++) {
    out.push(simulateRescheduling(curve, modified, spec, rng));
  }
  return out;
}

/** Panel default: on at a third of the period, halving the rate. */
export function defaultModifierSpec(duration: number): ModifierSpec {
  return { activatesAt: round1(duration / 3), factor: 0.5 };
}

function round1(v: number): number {
  return Math.round(v * 10) / 10;
}
