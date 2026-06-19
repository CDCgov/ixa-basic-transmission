import { describe, it, expect } from "vitest";
import type { InfectionRate } from "./infectionRate";
import {
  effectiveRateCurve,
  cumulativeRate,
  inverseCumulativeRate,
  sampledCumulative,
  expFromUniform,
  describeRate,
  rateFunctionDefs,
} from "./rateExplorer";

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
      duration: 14,
      scale: 2.4,
    };
    const pc = effectiveRateCurve(parametric);
    expect(pc.total).toBeCloseTo(2.4, 6);
    // Λ⁻¹ round-trips through Λ.
    const tau = inverseCumulativeRate(pc, 1.0);
    expect(tau).not.toBeNull();
    expect(cumulativeRate(pc, tau as number)).toBeCloseTo(1.0, 6);
  });

  it("library mean curve normalizes to scale", () => {
    const rate: InfectionRate = {
      type: "library",
      rates: [
        [
          [0, 0],
          [4, 1],
          [8, 0],
        ],
        [
          [0, 0],
          [2, 2],
          [6, 0],
        ],
      ],
      scale: 2,
    };
    expect(effectiveRateCurve(rate).total).toBeCloseTo(2, 6);
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
      duration: 12,
      scale: 3,
    });
    // r(t) is the (scaled) PDF; c(t) the (scaled) CDF; d(t) the quantile.
    // Both r and c note the kernel is truncated to [0, duration].
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
      duration: 15,
      scale: 2,
    });
    expect(weibull.r).toContain("Weibull PDF");
    expect(weibull.d.toLowerCase()).toContain("inverse");
    expect(weibull.d).toContain("Weibull");
    expect(weibull.d).toContain("truncated");

    const lognormal = rateFunctionDefs({
      type: "parametric",
      dist: { dist: "lognormal", mu: 1.4, sigma: 0.5 },
      duration: 15,
      scale: 2,
    });
    expect(lognormal.d.toLowerCase()).toContain("inverse");
    expect(lognormal.d).toContain("Lognormal");
    expect(lognormal.d).toContain("truncated");

    const constant = rateFunctionDefs({ type: "constant", value: 0.5, duration: 3 });
    expect(constant.r).toContain("flat");
    expect(constant.d).toContain("c / 0.5");
  });

  it("describeRate produces a human title + subtitle", () => {
    expect(describeRate({ type: "constant", value: 1, duration: 2 })).toEqual({
      title: "Constant rate",
      subtitle: "1 infection per day for 2 days",
    });
    // The distribution name is the (bold) title; params live on the subtitle.
    const gamma = describeRate({
      type: "parametric",
      dist: { dist: "gamma", shape: 3, scale: 1.5 },
      duration: 12,
      scale: 3,
    });
    expect(gamma.title).toBe("Gamma");
    expect(gamma.subtitle).toContain("shape=3, scale=1.5");
  });
});
