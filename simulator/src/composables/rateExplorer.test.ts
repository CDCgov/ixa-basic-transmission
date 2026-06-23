import { describe, it, expect } from "vitest";
import type { InfectionRate } from "./infectionRate";
import {
  effectiveRateCurve,
  libraryOverlay,
  cumulativeRate,
  inverseCumulativeRate,
  sampledCumulative,
  expFromUniform,
  simulateInfectionCount,
  simulateInfectionCounts,
  simulateInfectionTimes,
  simulateInfectionRuns,
  eventTimeHistogram,
  describeRate,
  rateFunctionDefs,
} from "./rateExplorer";

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

// Trapezoidal area under a polyline (x, y).
function trapz(x: number[], y: number[]): number {
  let a = 0;
  for (let i = 1; i < x.length; i++) {
    a += 0.5 * (y[i - 1] + y[i]) * (x[i] - x[i - 1]);
  }
  return a;
}

// Linear interpolation of a polyline (x, y) at xp — what a line chart draws.
function interp(x: number[], y: number[], xp: number): number {
  for (let i = 1; i < x.length; i++) {
    if (xp <= x[i]) {
      const span = x[i] - x[i - 1];
      if (span === 0) return y[i];
      return y[i - 1] + ((xp - x[i - 1]) / span) * (y[i] - y[i - 1]);
    }
  }
  return y[y.length - 1];
}

describe("rateExplorer", () => {
  it("constant rate: flat λ, total = value·duration, Λ⁻¹ is linear", () => {
    const rate: InfectionRate = { type: "constant", value: 1, duration: 2 };
    const c = effectiveRateCurve(rate);
    expect(c.duration).toBe(2);
    expect(c.total).toBeCloseTo(2, 12);
    expect(c.lambda.every((v) => v === 1)).toBe(true);
    // Λ(t) = value·t.
    expect(cumulativeRate(c, 1)).toBeCloseTo(1, 12);
    // Λ⁻¹(E) = E/value; matches the sketch's 0.414 → 0.414.
    expect(inverseCumulativeRate(c, 0.414)).toBeCloseTo(0.414, 12);
    expect(inverseCumulativeRate(c, 1.167)).toBeCloseTo(1.167, 12);
    // Past the support (E > total): recovered, no event.
    expect(inverseCumulativeRate(c, 2.5)).toBeNull();
  });

  it("constant rate with value≠1 scales Λ⁻¹", () => {
    const c = effectiveRateCurve({ type: "constant", value: 2, duration: 3 });
    expect(c.total).toBeCloseTo(6, 12);
    // Λ⁻¹(E) = E / 2.
    expect(inverseCumulativeRate(c, 3)).toBeCloseTo(1.5, 12);
  });

  it("empirical/parametric area-normalize so total = scale (R₀)", () => {
    const empirical: InfectionRate = {
      type: "empirical",
      points: [
        [0, 0],
        [2, 1],
        [4, 1.2],
        [6, 0.4],
        [8, 0],
      ],
      scale: 3,
    };
    expect(effectiveRateCurve(empirical).total).toBeCloseTo(3, 9);

    const parametric: InfectionRate = {
      type: "parametric",
      dist: { dist: "gamma", shape: 3, scale: 1.5 },
      scale: 2.4,
    };
    const pc = effectiveRateCurve(parametric);
    expect(pc.total).toBeCloseTo(2.4, 6);
    // Λ⁻¹ round-trips through Λ.
    const tau = inverseCumulativeRate(pc, 1.0);
    expect(tau).not.toBeNull();
    expect(cumulativeRate(pc, tau as number)).toBeCloseTo(1.0, 6);
  });

  it("library assigns one curve, scaled so the library's mean R₀ = scale", () => {
    const rate: InfectionRate = {
      type: "library",
      rates: [
        [
          [0, 0],
          [4, 1],
          [8, 0],
        ], // area 4
        [
          [0, 0],
          [2, 2],
          [6, 0],
        ], // area 6
      ],
      scale: 2,
    };
    // Each assigned curve keeps its relative size: factor = scale / meanArea
    // = 2 / 5. So per-person R₀ differs (4·0.4, 6·0.4)…
    expect(effectiveRateCurve(rate, 0).total).toBeCloseTo(1.6, 6);
    expect(effectiveRateCurve(rate, 1).total).toBeCloseTo(2.4, 6);
    // …and the per-curve totals average to scale — heterogeneity preserved.
    const mean =
      (effectiveRateCurve(rate, 0).total + effectiveRateCurve(rate, 1).total) /
      2;
    expect(mean).toBeCloseTo(2, 6);
    // Index wraps, so an out-of-range pick stays valid.
    expect(effectiveRateCurve(rate, 2).total).toBeCloseTo(1.6, 6);
  });

  it("libraryOverlay normalizes every curve and the red mean to R₀", () => {
    const rate: InfectionRate = {
      type: "library",
      rates: [
        [
          [0, 0],
          [4, 1],
          [8, 0],
        ], // area 4, peak 1
        [
          [0, 0],
          [2, 2],
          [6, 0],
        ], // area 6, peak 2
      ],
      scale: 2,
    };
    const o = libraryOverlay(rate);
    expect(o.curves).toHaveLength(2);
    // factor = scale / meanArea = 2 / 5 = 0.4 → peaks scale by 0.4.
    expect(Math.max(...o.curves[0].lambda)).toBeCloseTo(0.4, 9);
    expect(Math.max(...o.curves[1].lambda)).toBeCloseTo(0.8, 9);
    // The red mean integrates to R₀ (= scale).
    expect(trapz(o.mean.x, o.mean.data)).toBeCloseTo(2, 2);
    // Non-library rates have no overlay.
    expect(
      libraryOverlay({ type: "constant", value: 1, duration: 2 }).curves,
    ).toHaveLength(0);
  });

  it("event markers sit on the dense c(t) polyline (not the sparse chords)", () => {
    // The default empirical curve has a wide first segment where λ rises from
    // 0 — the chord between anchors overshoots the true quadratic c(t), so a
    // marker at (τ, E) lands well below the *sparse* render but on the *dense*
    // render. Pick an E that inverts into that first segment.
    const rate: InfectionRate = {
      type: "empirical",
      points: [
        [0, 0],
        [2, 1],
        [4, 1.2],
        [6, 0.4],
        [8, 0],
      ],
      scale: 3,
    };
    const curve = effectiveRateCurve(rate);
    const E = 0.2;
    const tau = inverseCumulativeRate(curve, E) as number;
    expect(tau).not.toBeNull();
    expect(tau).toBeLessThan(2); // lands in the first segment

    // Dense render passes through (τ, E) closely…
    const dense = sampledCumulative(curve);
    expect(interp(dense.x, dense.data, tau)).toBeCloseTo(E, 2);

    // …while the sparse anchor chord sits visibly higher (the bug).
    const sparseChord = interp(curve.x, curve.cum, tau);
    expect(sparseChord).toBeGreaterThan(E + 0.05);
  });

  it("expFromUniform inverts the Exp(1) CDF", () => {
    // u → -ln(1-u); u=0 → 0, and it's increasing.
    expect(expFromUniform(0)).toBeCloseTo(0, 12);
    expect(expFromUniform(1 - Math.exp(-1))).toBeCloseTo(1, 12);
  });

  it("rateFunctionDefs names the right PDF/CDF for the selected rate", () => {
    const gamma = rateFunctionDefs({
      type: "parametric",
      dist: { dist: "gamma", shape: 3, scale: 1.5 },
      scale: 3,
    });
    // r(t) is the (scaled) PDF; c(t) the (scaled) CDF; d(t) the quantile.
    // Both r and c note the kernel is truncated at its auto-derived support.
    expect(gamma.r).toContain("Gamma PDF");
    expect(gamma.c).toContain("Gamma CDF");
    expect(gamma.r).toContain("truncated");
    expect(gamma.c).toContain("truncated");
    expect(gamma.d.toLowerCase()).toContain("inverse");
    expect(gamma.d).toContain("Gamma");
    expect(gamma.d).toContain("truncated");

    // d(t) stays vague — just "the inverse of the truncated CDF", named per dist.
    const weibull = rateFunctionDefs({
      type: "parametric",
      dist: { dist: "weibull", shape: 2, scale: 5 },
      scale: 2,
    });
    expect(weibull.r).toContain("Weibull PDF");
    expect(weibull.d.toLowerCase()).toContain("inverse");
    expect(weibull.d).toContain("Weibull");
    expect(weibull.d).toContain("truncated");

    const lognormal = rateFunctionDefs({
      type: "parametric",
      dist: { dist: "lognormal", mu: 1.4, sigma: 0.5 },
      scale: 2,
    });
    expect(lognormal.d.toLowerCase()).toContain("inverse");
    expect(lognormal.d).toContain("Lognormal");
    expect(lognormal.d).toContain("truncated");

    const constant = rateFunctionDefs({ type: "constant", value: 0.5, duration: 3 });
    expect(constant.r).toContain("r(t) = 0.5");
    expect(constant.d).toContain("c / 0.5");
  });

  it("describeRate produces a human title", () => {
    expect(describeRate({ type: "constant", value: 1, duration: 2 })).toEqual({
      title: "Constant rate of 1 infection per day for 2 days",
    });
    // The distribution name and its params share one title line.
    const gamma = describeRate({
      type: "parametric",
      dist: { dist: "gamma", shape: 3, scale: 1.5 },
      scale: 3,
    });
    expect(gamma.title).toContain("Gamma");
    expect(gamma.title).toContain("shape=3, scale=1.5");
  });
});

describe("rateExplorer offspring simulation", () => {
  it("zero-area curve generates no infections", () => {
    const c = effectiveRateCurve({
      type: "empirical",
      points: [
        [0, 0],
        [1, 0],
      ],
      scale: 0,
    });
    expect(simulateInfectionCount(c, mulberry32(1))).toBe(0);
  });

  it("is deterministic for a fixed rng", () => {
    const c = effectiveRateCurve({ type: "constant", value: 1, duration: 4 });
    expect(simulateInfectionCounts(c, 50, mulberry32(7))).toEqual(
      simulateInfectionCounts(c, 50, mulberry32(7)),
    );
  });

  it("mean infection count ≈ R₀ (curve total)", () => {
    // Constant value 0.6 over 5 days → R₀ = 3 (a homogeneous Poisson process).
    const c = effectiveRateCurve({ type: "constant", value: 0.6, duration: 5 });
    const counts = simulateInfectionCounts(c, 5000, mulberry32(42));
    const mean = counts.reduce((a, b) => a + b, 0) / counts.length;
    expect(mean).toBeGreaterThan(2.8);
    expect(mean).toBeLessThan(3.2);
  });

  it("the empirical curve's mean count tracks its R₀ too", () => {
    const c = effectiveRateCurve({
      type: "empirical",
      points: [
        [0, 0],
        [2, 1],
        [4, 1.2],
        [6, 0.4],
        [8, 0],
      ],
      scale: 2.5,
    });
    const counts = simulateInfectionCounts(c, 5000, mulberry32(99));
    const mean = counts.reduce((a, b) => a + b, 0) / counts.length;
    expect(mean).toBeGreaterThan(2.3);
    expect(mean).toBeLessThan(2.7);
  });

  it("simulateInfectionTimes returns ascending in-range times; length = count", () => {
    const c = effectiveRateCurve({
      type: "empirical",
      points: [
        [0, 0],
        [2, 1],
        [4, 1.2],
        [6, 0.4],
        [8, 0],
      ],
      scale: 3,
    });
    // Same seed → the count equals the number of times (count delegates to it).
    const times = simulateInfectionTimes(c, mulberry32(5));
    expect(times.length).toBe(simulateInfectionCount(c, mulberry32(5)));
    for (let i = 1; i < times.length; i++) {
      expect(times[i]).toBeGreaterThanOrEqual(times[i - 1]);
    }
    for (const t of times) {
      expect(t).toBeGreaterThanOrEqual(0);
      expect(t).toBeLessThanOrEqual(c.duration);
    }
  });

  it("simulateInfectionRuns: per-run lengths match simulateInfectionCounts", () => {
    const c = effectiveRateCurve({ type: "constant", value: 0.6, duration: 5 });
    const runs = simulateInfectionRuns(c, 40, mulberry32(11));
    expect(runs).toHaveLength(40);
    expect(runs.map((r) => r.length)).toEqual(
      simulateInfectionCounts(c, 40, mulberry32(11)),
    );
  });
});

describe("rateExplorer event-time histogram", () => {
  it("bins times over [0, duration]; right edge lands in the last bin", () => {
    // duration 10, 5 bins → width 2. floor(t/2): 0.5,1.5→bin0; 2.5→bin1; 9.9→bin4.
    // The right edge t=10 clamps into the last bin; out-of-range is dropped.
    const h = eventTimeHistogram([0.5, 1.5, 2.5, 9.9, 10, -1, 11], 10, 5);
    expect(h.counts).toEqual([2, 1, 0, 0, 2]);
    expect(h.total).toBe(5);
    expect(h.centers).toEqual([1, 3, 5, 7, 9]);
    expect(h.categories).toEqual(["1", "3", "5", "7", "9"]);
    expect(h.binWidth).toBe(2);
    // Percentages are counts / total · 100 and sum to 100.
    expect(h.percentages).toEqual([40, 20, 0, 0, 40]);
    expect(h.percentages.reduce((a, b) => a + b, 0)).toBeCloseTo(100, 12);
  });

  it("empty sample → all-zero bins (no divide-by-zero)", () => {
    const h = eventTimeHistogram([], 8, 4);
    expect(h.counts).toEqual([0, 0, 0, 0]);
    expect(h.percentages).toEqual([0, 0, 0, 0]);
    expect(h.total).toBe(0);
  });

  it("the histogram's shape tracks the rate curve (more mass where r is higher)", () => {
    // A rising-then-falling empirical curve: the middle bins should collect
    // more sampled times than the tails.
    const rate: InfectionRate = {
      type: "empirical",
      points: [
        [0, 0],
        [4, 2],
        [8, 0],
      ],
      scale: 4,
    };
    const c = effectiveRateCurve(rate);
    const times = simulateInfectionRuns(c, 400, mulberry32(7)).flat();
    const h = eventTimeHistogram(times, c.duration, 4); // bins over [0,8], width 2
    // The peak is at τ=4 (boundary of bins 1 and 2); both interior bins should
    // hold more than the first bin (which starts at λ=0).
    expect(h.counts[1] + h.counts[2]).toBeGreaterThan(h.counts[0] + h.counts[3]);
  });
});
