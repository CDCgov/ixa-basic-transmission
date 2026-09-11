import { describe, it, expect } from "vitest";
import type { InfectionRate } from "./infectionRate";
import { effectiveRateCurve, cumulativeRate } from "./rateExplorer";
import {
  modifierActive,
  thinningApplies,
  modifiedRateAt,
  modifiedRateCurve,
  thinningAttempts,
  simulateThinning,
  simulateThinningRuns,
  reschedulingAttempts,
  needsRedraw,
  simulateRescheduling,
  simulateReschedulingRuns,
  defaultModifierSpec,
} from "./modifierExplorer";

// Deterministic uniform PRNG (mulberry32) so simulation tests are reproducible.
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const constant: InfectionRate = { type: "constant", value: 1, duration: 4 };
const triangle: InfectionRate = {
  type: "empirical",
  points: [
    [0, 0],
    [2, 1],
    [6, 0],
  ],
  scale: 3,
};

describe("modifiedRateCurve", () => {
  it("scales the rate by f after the activation time and keeps it before", () => {
    const base = effectiveRateCurve(constant);
    const spec = { activatesAt: 1, factor: 0.25 };
    const mod = modifiedRateCurve(base, spec);
    expect(mod.duration).toBe(4);
    // 1·1 + 0.25·3 = 1.75
    expect(mod.total).toBeCloseTo(1.75, 12);
    expect(cumulativeRate(mod, 1)).toBeCloseTo(1, 12);
    expect(cumulativeRate(mod, 4)).toBeCloseTo(1.75, 12);
    expect(modifiedRateAt(base, spec, 0.5)).toBe(1);
    expect(modifiedRateAt(base, spec, 1)).toBe(0.25);
  });

  it("is a jump discontinuity encoded as two anchors at t_m", () => {
    const base = effectiveRateCurve(triangle);
    const mod = modifiedRateCurve(base, { activatesAt: 3, factor: 0.5 });
    const i = mod.x.indexOf(3);
    expect(mod.x[i + 1]).toBe(3);
    expect(mod.lambda[i + 1]).toBeCloseTo(mod.lambda[i] * 0.5, 12);
    // Area = area before + f · area after.
    const before = cumulativeRate(base, 3);
    expect(mod.total).toBeCloseTo(before + 0.5 * (base.total - before), 12);
  });

  it("is the base curve when the modifier is inert", () => {
    const base = effectiveRateCurve(constant);
    expect(modifierActive(base, { activatesAt: 4, factor: 0.5 })).toBe(false);
    expect(modifierActive(base, { activatesAt: 1, factor: 1 })).toBe(false);
    expect(modifiedRateCurve(base, { activatesAt: 9, factor: 0.5 }).total).toBe(
      base.total,
    );
    expect(modifiedRateCurve(base, { activatesAt: 0, factor: 0.5 }).total).toBeCloseTo(
      base.total / 2,
      12,
    );
  });
});

describe("a modifier that raises the rate (f > 1)", () => {
  const base = effectiveRateCurve(constant); // λ = 1 on [0, 4]
  const spec = { activatesAt: 2, factor: 1.5 };

  it("is active and grows the modified area", () => {
    expect(modifierActive(base, spec)).toBe(true);
    const mod = modifiedRateCurve(base, spec);
    // 1·2 + 1.5·2 = 5
    expect(mod.total).toBeCloseTo(5, 12);
    expect(modifiedRateAt(base, spec, 3)).toBe(1.5);
  });

  it("rules thinning out; rescheduling still works", () => {
    expect(thinningApplies(spec)).toBe(false);
    expect(thinningApplies({ activatesAt: 2, factor: 1 })).toBe(true);
    expect(thinningAttempts(base, spec, [{ e: 0.5, u: 0.1 }])).toEqual([]);
    expect(simulateThinning(base, spec, mulberry32(9))).toEqual({
      accepted: [],
      rejected: [],
    });
    const mod = modifiedRateCurve(base, spec);
    const attempts = reschedulingAttempts(base, mod, spec, [
      2.5, // τ = 2.5 ≥ t_m → cancelled at t_m
      0.75, // from t_m on 1.5·r: τ = 2 + 0.75/1.5 = 2.5
    ]);
    expect(attempts.map((a) => a.outcome)).toEqual(["cancelled", "event"]);
    expect(attempts[1].tau).toBeCloseTo(2.5, 12);
    const runs = simulateReschedulingRuns(base, mod, spec, 4000, mulberry32(10));
    const mean = runs.reduce((a, r) => a + r.events.length, 0) / runs.length;
    expect(mean).toBeCloseTo(5, 0);
  });
});

describe("thinningAttempts", () => {
  const base = effectiveRateCurve(constant); // λ = 1 on [0, 4]
  const spec = { activatesAt: 2, factor: 0.5 };

  it("accepts before activation without consuming a uniform", () => {
    const [a] = thinningAttempts(base, spec, [{ e: 0.5, u: 0.99 }]);
    expect(a.tau).toBeCloseTo(0.5, 12);
    expect(a.pAccept).toBe(1);
    expect(a.u).toBeNull();
    expect(a.outcome).toBe("accepted");
  });

  it("after activation accepts iff u < f, and keeps walking from a rejection", () => {
    const attempts = thinningAttempts(base, spec, [
      { e: 2.5, u: 0.7 }, // τ = 2.5 ≥ t_m, u ≥ f → rejected
      { e: 0.5, u: 0.1 }, // τ = 3.0, u < f → accepted
    ]);
    expect(attempts[0].outcome).toBe("rejected");
    expect(attempts[0].pAccept).toBe(0.5);
    expect(attempts[0].u).toBe(0.7);
    expect(attempts[1].tau).toBeCloseTo(3, 12);
    expect(attempts[1].outcome).toBe("accepted");
  });

  it("marks the exhausting draw as recovered and stops", () => {
    const attempts = thinningAttempts(base, spec, [
      { e: 5, u: 0.1 },
      { e: 1, u: 0.1 },
    ]);
    expect(attempts).toHaveLength(1);
    expect(attempts[0].outcome).toBe("recovered");
    expect(attempts[0].tau).toBeNull();
  });
});

describe("reschedulingAttempts", () => {
  const base = effectiveRateCurve(constant);
  const spec = { activatesAt: 2, factor: 0.5 };
  const mod = modifiedRateCurve(base, spec);

  it("cancels the forecast that crosses t_m and restarts on f·r from t_m", () => {
    const attempts = reschedulingAttempts(base, mod, spec, [
      1, // τ = 1 < t_m → event on r(t)
      1.5, // τ = 2.5 ≥ t_m → cancelled
      0.5, // from t_m on f·r: Λ_mod(2) + 0.5 → τ = 2 + 0.5/0.5 = 3
    ]);
    expect(attempts.map((a) => a.outcome)).toEqual(["event", "cancelled", "event"]);
    expect(attempts[1].onModified).toBe(false);
    expect(attempts[1].tau).toBeCloseTo(2.5, 12);
    expect(attempts[2].onModified).toBe(true);
    expect(attempts[2].from).toBe(2);
    expect(attempts[2].tau).toBeCloseTo(3, 12);
    // The redraw is the one made at t_m, and only that one.
    expect(attempts.map((a) => a.rescheduled)).toEqual([false, false, true]);
    expect(needsRedraw(attempts.slice(0, 2))).toBe(true);
    expect(needsRedraw(attempts)).toBe(false);
  });

  it("an exhausted pre-activation forecast is 'none', not a cancellation", () => {
    const attempts = reschedulingAttempts(base, mod, spec, [10, 0.5]);
    expect(attempts[0].outcome).toBe("none");
    expect(attempts[0].tau).toBeNull();
    expect(needsRedraw(attempts.slice(0, 1))).toBe(true);
    expect(attempts[1].outcome).toBe("event");
    expect(attempts[1].rescheduled).toBe(true);
    expect(attempts[1].tau).toBeCloseTo(3, 12);
    // Nothing was pending, so the simulator counts no cancellation.
    const run = simulateRescheduling(base, mod, spec, () => 1 - Math.exp(-10));
    expect(run.cancelled).toBe(0);
  });

  it("never cancels when the modifier is inert", () => {
    const inert = { activatesAt: 9, factor: 0.5 };
    const attempts = reschedulingAttempts(base, modifiedRateCurve(base, inert), inert, [
      3.5, 1,
    ]);
    expect(attempts.map((a) => a.outcome)).toEqual(["event", "recovered"]);
  });
});

describe("the two methods sample the same process", () => {
  it("both mean offspring counts converge to the modified area", () => {
    const base = effectiveRateCurve(triangle); // area 3 over [0, 6]
    const spec = { activatesAt: 2, factor: 0.3 };
    const mod = modifiedRateCurve(base, spec);
    const runs = 4000;
    const thin = simulateThinningRuns(base, spec, runs, mulberry32(1));
    const resched = simulateReschedulingRuns(base, mod, spec, runs, mulberry32(2));
    const thinMean = thin.reduce((a, r) => a + r.accepted.length, 0) / runs;
    const reschedMean = resched.reduce((a, r) => a + r.events.length, 0) / runs;
    // Poisson counts with mean ≈ 1.6 → sd of the mean over 4000 runs ≈ 0.02.
    expect(thinMean).toBeCloseTo(mod.total, 1);
    expect(reschedMean).toBeCloseTo(mod.total, 1);
    expect(Math.abs(thinMean - reschedMean)).toBeLessThan(0.1);
  });

  it("both event-time distributions follow f·r(t) after activation", () => {
    const base = effectiveRateCurve(constant); // λ = 1 on [0, 4]
    const spec = { activatesAt: 2, factor: 0.25 };
    const mod = modifiedRateCurve(base, spec);
    const runs = 4000;
    const thinTimes = simulateThinningRuns(base, spec, runs, mulberry32(3)).flatMap(
      (r) => r.accepted,
    );
    const reschedTimes = simulateReschedulingRuns(
      base,
      mod,
      spec,
      runs,
      mulberry32(4),
    ).flatMap((r) => r.events);
    // Expected events per run: 2 before t_m, 0.25·2 = 0.5 after → 20% after.
    const afterShare = (ts: number[]) => ts.filter((t) => t >= 2).length / ts.length;
    expect(afterShare(thinTimes)).toBeCloseTo(0.2, 1);
    expect(afterShare(reschedTimes)).toBeCloseTo(0.2, 1);
  });

  it("thinning rejects ≈ (1 − f) of post-activation forecasts; rescheduling cancels ≤ 1 per individual", () => {
    const base = effectiveRateCurve(constant);
    const spec = { activatesAt: 2, factor: 0.25 };
    const mod = modifiedRateCurve(base, spec);
    const rng = mulberry32(5);
    const thin = simulateThinning(base, spec, rng);
    expect(thin.accepted.every((t) => t >= 0 && t <= 4)).toBe(true);
    const runs = simulateThinningRuns(base, spec, 2000, mulberry32(6));
    const after = runs.flatMap((r) => [
      ...r.accepted.filter((t) => t >= 2).map(() => 1),
      ...r.rejected.map(() => 0),
    ]);
    const acceptRate = after.reduce((a, b) => a + b, 0) / after.length;
    expect(acceptRate).toBeCloseTo(0.25, 1);

    const resched = simulateReschedulingRuns(base, mod, spec, 500, mulberry32(7));
    expect(resched.every((r) => r.cancelled <= 1)).toBe(true);
    // A run cancels iff a forecast was pending past t_m: P(no event on r(t)
    // reaching past 4 before 2) is small but nonzero, so most runs cancel.
    const share = resched.filter((r) => r.cancelled === 1).length / resched.length;
    expect(share).toBeGreaterThan(0.8);
    const one = simulateRescheduling(base, mod, spec, mulberry32(8));
    expect(one.events.every((t) => t >= 0 && t <= 4)).toBe(true);
  });
});

describe("defaultModifierSpec", () => {
  it("activates a third of the way through and halves the rate", () => {
    expect(defaultModifierSpec(3)).toEqual({ activatesAt: 1, factor: 0.5 });
  });
});
