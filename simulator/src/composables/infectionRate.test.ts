import { describe, it, expect } from "vitest";
import {
  type InfectionRate,
  type ParametricDist,
  parametricKernel,
  parametricPoints,
  expectedR0,
  withRateType,
  normalizeInfectionRate,
  PARAMETRIC_GRID,
  DEFAULT_PARAMETRIC_DURATION,
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

  it("samples a dense grid spanning [0, duration]", () => {
    const dist: ParametricDist = { dist: "gamma", shape: 2, scale: 1.5 };
    const pts = parametricPoints(dist, 12);
    expect(pts).toHaveLength(PARAMETRIC_GRID);
    expect(pts[0][0]).toBe(0);
    expect(pts[pts.length - 1][0]).toBeCloseTo(12, 12);
    // Strictly increasing times.
    expect(pts.every((p, i) => i === 0 || p[0] > pts[i - 1][0])).toBe(true);
  });

  it("R₀ under random mixing equals scale", () => {
    const rate: InfectionRate = {
      type: "parametric",
      dist: { dist: "gamma", shape: 3, scale: 1.5 },
      duration: 14,
      scale: 2.4,
    };
    expect(expectedR0(rate, [])).toBeCloseTo(2.4, 12);
  });

  it("round-trips through withRateType, preserving the infectious period", () => {
    const empirical = withRateType(
      { type: "constant", value: 0.5, duration: 7 },
      "empirical",
    );
    const parametric = withRateType(empirical, "parametric");
    expect(parametric.type).toBe("parametric");
    // Switching back to constant carries the parametric duration over.
    const back = withRateType(parametric, "constant");
    expect(back.type).toBe("constant");
    if (parametric.type === "parametric" && back.type === "constant") {
      expect(back.duration).toBe(parametric.duration);
    }
  });

  it("withRateType seeds a sensible default duration", () => {
    const parametric = withRateType(
      { type: "library", rates: [], scale: 3 },
      "parametric",
    );
    expect(parametric.type).toBe("parametric");
    if (parametric.type === "parametric") {
      expect(parametric.duration).toBe(DEFAULT_PARAMETRIC_DURATION);
    }
  });

  it("normalizeInfectionRate fills a missing scale with 1.0", () => {
    const raw = {
      type: "parametric",
      dist: { dist: "weibull", shape: 2, scale: 5 },
      duration: 15,
    } as unknown as InfectionRate;
    const normalized = normalizeInfectionRate(raw);
    expect(normalized.type).toBe("parametric");
    if (normalized.type === "parametric") {
      expect(normalized.scale).toBe(1.0);
      expect(normalized.dist).toEqual({ dist: "weibull", shape: 2, scale: 5 });
    }
  });
});
