import { describe, it, expect } from "vitest";
import {
  type InfectionRate,
  type ParametricDist,
  parametricKernel,
  parametricPoints,
  parametricSupport,
  expectedR0,
  withRateType,
  normalizeInfectionRate,
  PARAMETRIC_GRID,
  DEFAULT_PARAMETRIC_DIST,
} from "./infectionRate";

describe("parametric infection rate", () => {
  it("kernels match the closed forms (mirrors Rust)", () => {
    // Gamma(shape=1): e^{-t/scale}; at t=2, scale=2 → e^{-1}.
    expect(
      parametricKernel({ dist: "gamma", shape: 1, scale: 2 }, 2),
    ).toBeCloseTo(Math.exp(-1), 12);
    // Lognormal(0,1): (1/t)·e^{-(ln t)²/2}; at t=1 → 1.
    expect(
      parametricKernel({ dist: "lognormal", mu: 0, sigma: 1 }, 1),
    ).toBeCloseTo(1, 12);
    // Weibull(shape=2, scale=1): t·e^{-t²}; at t=1 → e^{-1}.
    expect(
      parametricKernel({ dist: "weibull", shape: 2, scale: 1 }, 1),
    ).toBeCloseTo(Math.exp(-1), 12);
    // Zero for τ ≤ 0.
    expect(parametricKernel({ dist: "gamma", shape: 2, scale: 1 }, 0)).toBe(0);
    expect(parametricKernel({ dist: "gamma", shape: 2, scale: 1 }, -1)).toBe(0);
  });

  it("samples a dense grid over the auto-derived support", () => {
    const dist: ParametricDist = { dist: "gamma", shape: 2, scale: 1.5 };
    const pts = parametricPoints(dist);
    expect(pts).toHaveLength(PARAMETRIC_GRID);
    expect(pts[0][0]).toBe(0);
    // Last anchor lands at the distribution's own support (mean 3 → well past).
    expect(pts[pts.length - 1][0]).toBeCloseTo(parametricSupport(dist), 9);
    expect(pts[pts.length - 1][0]).toBeGreaterThan(3);
    // Strictly increasing times.
    expect(pts.every((p, i) => i === 0 || p[0] > pts[i - 1][0])).toBe(true);
  });

  it("auto-support captures ~all the mass and is finite", () => {
    const dist: ParametricDist = { dist: "weibull", shape: 2, scale: 5 };
    const support = parametricSupport(dist);
    expect(Number.isFinite(support)).toBe(true);
    // The lowered grid's trapezoid area ≈ the kernel's far-bound integral.
    const pts = parametricPoints(dist);
    const captured = pts
      .slice(1)
      .reduce(
        (acc, p, i) => acc + 0.5 * (pts[i][1] + p[1]) * (p[0] - pts[i][0]),
        0,
      );
    let full = 0;
    let prev = parametricKernel(dist, 0);
    const h = 60 / 20000;
    for (let i = 1; i <= 20000; i++) {
      const v = parametricKernel(dist, h * i);
      full += 0.5 * (prev + v) * h;
      prev = v;
    }
    expect(captured / full).toBeGreaterThan(0.99);
  });

  it("R₀ under random mixing equals scale", () => {
    const rate: InfectionRate = {
      type: "parametric",
      dist: { dist: "gamma", shape: 3, scale: 1.5 },
      scale: 2.4,
    };
    expect(expectedR0(rate, [])).toBeCloseTo(2.4, 12);
  });

  it("round-trips through withRateType", () => {
    const empirical = withRateType(
      { type: "constant", value: 0.5, duration: 7 },
      "empirical",
    );
    const parametric = withRateType(empirical, "parametric");
    expect(parametric.type).toBe("parametric");
    // Switching back to constant yields a valid constant (parametric has no
    // duration to carry, so it falls back to the default mean period).
    const back = withRateType(parametric, "constant");
    expect(back.type).toBe("constant");
    if (back.type === "constant") {
      expect(back.duration).toBeGreaterThan(0);
    }
  });

  it("withRateType seeds the default distribution", () => {
    const parametric = withRateType(
      { type: "library", rates: [], scale: 3 },
      "parametric",
    );
    expect(parametric.type).toBe("parametric");
    if (parametric.type === "parametric") {
      expect(parametric.dist).toEqual(DEFAULT_PARAMETRIC_DIST);
    }
  });

  it("normalizeInfectionRate fills a missing scale with 1.0", () => {
    const raw = {
      type: "parametric",
      dist: { dist: "weibull", shape: 2, scale: 5 },
    } as unknown as InfectionRate;
    const normalized = normalizeInfectionRate(raw);
    expect(normalized.type).toBe("parametric");
    if (normalized.type === "parametric") {
      expect(normalized.scale).toBe(1.0);
      expect(normalized.dist).toEqual({ dist: "weibull", shape: 2, scale: 5 });
    }
  });
});
