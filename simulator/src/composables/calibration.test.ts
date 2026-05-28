import { describe, it, expect } from "vitest";
import {
  computeThreshold,
  parseTargetCsv,
  weightedHistogram,
  totalWeight,
  acceptanceRatio,
  defaultConfig,
  withR0,
  cumulativeToIncidence,
  type Particle,
} from "./calibration";
import type { InfectionRate } from "./infectionRate";

function particle(distance: number, weight = 1): Particle {
  return { r0: 0, initialInfections: 0, weight, distance, trajectory: [] };
}

describe("computeThreshold", () => {
  it("returns Infinity for empty particle list", () => {
    expect(computeThreshold([], 0.5)).toBe(Infinity);
  });

  it("picks the floor(relativeError * n) - 1 element of the sorted distances", () => {
    // Mirrors def_abc_smc/src/lib.rs:50:
    //   floor(rel * n) - 1 indexed into sorted ascending.
    const ps = [10, 20, 30, 40, 50].map((d) => particle(d));
    expect(computeThreshold(ps, 0.5)).toBe(20); // floor(2.5)-1 = 1 → 20
    expect(computeThreshold(ps, 0.3)).toBe(10); // floor(1.5)-1 = 0 → 10
    expect(computeThreshold(ps, 1.0)).toBe(50); // floor(5)-1 = 4 → 50
  });

  it("clamps the index to >= 0", () => {
    // relativeError small enough that floor(rel*n)-1 would go negative.
    const ps = [10, 20, 30].map((d) => particle(d));
    expect(computeThreshold(ps, 0.1)).toBe(10); // would be -1; clamps to 0
  });

  it("uses ascending order regardless of input order", () => {
    const ps = [50, 10, 30].map((d) => particle(d));
    expect(computeThreshold(ps, 1.0)).toBe(50); // sorted [10,30,50], idx 2
  });
});

describe("parseTargetCsv", () => {
  it("parses a header + two-column rows", () => {
    const csv = "time,incident_cases\n1,5\n2,10\n3,15\n";
    expect(parseTargetCsv(csv)).toEqual([5, 10, 15]);
  });

  it("parses single-column rows", () => {
    expect(parseTargetCsv("5\n10\n15")).toEqual([5, 10, 15]);
  });

  it("auto-detects header presence by first row's numericness", () => {
    // No header — first row is numeric.
    expect(parseTargetCsv("1,5\n2,10")).toEqual([5, 10]);
    // Header — first row's first cell is non-numeric.
    expect(parseTargetCsv("day,cases\n1,5\n2,10")).toEqual([5, 10]);
  });

  it("rounds non-integer values", () => {
    expect(parseTargetCsv("1.4\n2.6")).toEqual([1, 3]);
  });

  it("handles CRLF and blank lines", () => {
    expect(parseTargetCsv("1\r\n2\r\n\r\n3\r\n")).toEqual([1, 2, 3]);
  });

  it("throws on negative or non-numeric values", () => {
    expect(() => parseTargetCsv("1\n-2\n3")).toThrow(/Invalid incidence/);
    expect(() => parseTargetCsv("1\nfoo\n3")).toThrow(/Invalid incidence/);
  });

  it("returns empty array for empty input", () => {
    expect(parseTargetCsv("")).toEqual([]);
    expect(parseTargetCsv("\n\n")).toEqual([]);
  });
});

describe("weightedHistogram", () => {
  it("returns empty bins for binCount <= 0 or empty range", () => {
    expect(weightedHistogram([1, 2], [1, 1], 0, 0, 10)).toEqual([]);
    expect(weightedHistogram([1, 2], [1, 1], 5, 10, 10)).toEqual([]);
  });

  it("computes evenly spaced bin centers between lo and hi", () => {
    const bins = weightedHistogram([], [], 4, 0, 4);
    expect(bins.map((b) => b.center)).toEqual([0.5, 1.5, 2.5, 3.5]);
  });

  it("assigns each value to its bin and sums weights", () => {
    // 4 bins of width 1 over [0, 4): centers 0.5, 1.5, 2.5, 3.5.
    // 0.5 → bin 0, 1.0 → bin 1, 2.5 → bin 2, 3.99 → bin 3.
    const bins = weightedHistogram(
      [0.5, 1.0, 2.5, 3.99],
      [0.1, 0.2, 0.3, 0.4],
      4,
      0,
      4,
    );
    expect(bins.map((b) => b.weight)).toEqual([0.1, 0.2, 0.3, 0.4]);
  });

  it("clamps the top edge into the last bin", () => {
    // value == hi → would compute idx == binCount; clamps to binCount-1.
    const bins = weightedHistogram([10], [1], 5, 0, 10);
    expect(bins[4].weight).toBe(1);
  });

  it("drops out-of-range and non-finite values silently", () => {
    const bins = weightedHistogram(
      [-1, 100, NaN, Infinity, 1.5],
      [1, 1, 1, 1, 7],
      4,
      0,
      4,
    );
    const total = bins.reduce((s, b) => s + b.weight, 0);
    expect(total).toBe(7);
  });

  it("uses 0 weight when the weights array is shorter than values", () => {
    const bins = weightedHistogram([0.5, 1.5, 2.5], [1, 1], 4, 0, 4);
    expect(bins.reduce((s, b) => s + b.weight, 0)).toBe(2);
  });
});

describe("totalWeight", () => {
  it("returns 0 for empty input", () => {
    expect(totalWeight([])).toBe(0);
  });

  it("sums weights", () => {
    expect(totalWeight([particle(0, 0.5), particle(0, 0.3), particle(0, 0.2)])).toBeCloseTo(1.0);
  });
});

describe("withR0", () => {
  it("maps r0 to value/duration for Constant rate", () => {
    const rate: InfectionRate = { type: "constant", value: 0, duration: 3 };
    const next = withR0(rate, 1.5);
    expect(next).toEqual({ type: "constant", value: 0.5, duration: 3 });
  });

  it("maps r0 to scale for Empirical rate (preserves points)", () => {
    const points: [number, number][] = [
      [0, 0],
      [2, 1],
      [4, 0],
    ];
    const rate: InfectionRate = { type: "empirical", points, scale: 1 };
    const next = withR0(rate, 2.5);
    expect(next).toEqual({ type: "empirical", points, scale: 2.5 });
  });

  it("maps r0 to scale for Library rate (preserves rates)", () => {
    const rates: [number, number][][] = [[[0, 0], [2, 1]]];
    const rate: InfectionRate = { type: "library", rates, scale: 1 };
    const next = withR0(rate, 4);
    expect(next).toEqual({ type: "library", rates, scale: 4 });
  });
});

describe("cumulativeToIncidence", () => {
  it("returns empty for input length < 2", () => {
    expect(cumulativeToIncidence([])).toEqual([]);
    expect(cumulativeToIncidence([5])).toEqual([]);
  });

  it("diffs each consecutive pair", () => {
    expect(cumulativeToIncidence([0, 1, 3, 7])).toEqual([1, 2, 4]);
  });

  it("clamps negative diffs to 0 (cumulative is expected to be non-decreasing)", () => {
    expect(cumulativeToIncidence([0, 5, 3, 8])).toEqual([5, 0, 5]);
  });

  it("rounds fractional diffs to nearest integer", () => {
    expect(cumulativeToIncidence([0, 0.4, 1.5])).toEqual([0, 1]);
  });
});

describe("acceptanceRatio", () => {
  it("returns 0 when no attempts", () => {
    expect(acceptanceRatio(0, 0)).toBe(0);
    expect(acceptanceRatio(5, 0)).toBe(0);
  });

  it("returns nAccepted / nAttempts", () => {
    expect(acceptanceRatio(20, 100)).toBe(0.2);
    expect(acceptanceRatio(100, 100)).toBe(1.0);
  });
});

describe("defaultConfig", () => {
  it("produces a valid config that round-trips through JSON", () => {
    const c = defaultConfig();
    expect(c.modelContext.population).toBeGreaterThan(0);
    expect(c.priors.r0Lo).toBeLessThan(c.priors.r0Hi);
    expect(c.priors.initialInfectionsLo).toBeLessThanOrEqual(
      c.priors.initialInfectionsHi,
    );
    expect(c.stages.every((s) => s > 0 && s < 1)).toBe(true);
    // Round-trip — guards against any Vue Proxy contamination.
    const rt = JSON.parse(JSON.stringify(c));
    expect(rt).toEqual(c);
  });
});
