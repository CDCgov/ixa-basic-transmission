import { describe, it, expect } from "vitest";
import {
  computeThreshold,
  parseTargetCsv,
  weightedHistogram,
  weightedKde,
  totalWeight,
  acceptanceRatio,
  defaultConfig,
  withR0,
  cumulativeToIncidence,
  seedObservationHistogram,
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

describe("weightedKde", () => {
  // Trapezoidal integration on a regular grid.
  function area(x: number[], y: number[]): number {
    let s = 0;
    for (let i = 1; i < x.length; i++) {
      s += 0.5 * (y[i] + y[i - 1]) * (x[i] - x[i - 1]);
    }
    return s;
  }

  it("returns empty for invalid inputs", () => {
    expect(weightedKde([], [], 0, 1, 10)).toEqual({ x: [], y: [] });
    expect(weightedKde([1], [1], 1, 0, 10)).toEqual({ x: [], y: [] });
    expect(weightedKde([1], [1], 0, 1, 1)).toEqual({ x: [], y: [] });
  });

  it("output grid has nGrid points spanning [lo, hi]", () => {
    const r = weightedKde([0.5], [1], 0, 1, 11);
    expect(r.x).toHaveLength(11);
    expect(r.y).toHaveLength(11);
    expect(r.x[0]).toBeCloseTo(0);
    expect(r.x[10]).toBeCloseTo(1);
    expect(r.x[5]).toBeCloseTo(0.5);
  });

  it("integrates to approximately 1 over a wide grid (uniform weights)", () => {
    // 200 draws from N(50, 5²), uniform weights. KDE area on [10, 90]
    // should capture ~all the mass.
    const rng = mulberry(42);
    const values = Array.from({ length: 200 }, () => 50 + 5 * gaussian(rng));
    const w = Array.from({ length: 200 }, () => 1 / 200);
    const r = weightedKde(values, w, 10, 90, 200);
    expect(area(r.x, r.y)).toBeCloseTo(1, 1);
  });

  it("integrates to approximately 1 with non-uniform weights", () => {
    // Two clusters; one weighted heavier. Total weight = 1.
    const values = [0, 0.1, 0.2, 5, 5.1, 5.2];
    const w = [0.05, 0.05, 0.05, 0.3, 0.3, 0.25];
    const r = weightedKde(values, w, -5, 10, 200);
    expect(area(r.x, r.y)).toBeCloseTo(1, 1);
  });

  it("peaks near the data center for tightly clustered samples", () => {
    const values = Array.from({ length: 50 }, (_, i) => 10 + 0.01 * i);
    const w = Array.from({ length: 50 }, () => 1 / 50);
    const r = weightedKde(values, w, 5, 15, 201);
    let argmax = 0;
    for (let i = 1; i < r.y.length; i++) {
      if (r.y[i] > r.y[argmax]) argmax = i;
    }
    expect(r.x[argmax]).toBeGreaterThan(9);
    expect(r.x[argmax]).toBeLessThan(11);
  });

  it("falls back to σ when IQR is degenerate (all values tied)", () => {
    // All particles tied at 5; IQR = 0, so robust scale must fall back
    // to σ (which itself is ~0). Output should be finite (no NaN/Inf)
    // and concentrate mass near 5.
    const values = [5, 5, 5, 5, 5];
    const w = [0.2, 0.2, 0.2, 0.2, 0.2];
    const r = weightedKde(values, w, 0, 10, 51);
    for (const y of r.y) {
      expect(Number.isFinite(y)).toBe(true);
    }
    // The peak should be near 5 (within one grid step).
    let argmax = 0;
    for (let i = 1; i < r.y.length; i++) {
      if (r.y[i] > r.y[argmax]) argmax = i;
    }
    expect(Math.abs(r.x[argmax] - 5)).toBeLessThan(0.3);
  });

  it("is deterministic for the same inputs", () => {
    const values = [1, 2, 3, 4, 5];
    const w = [0.1, 0.2, 0.4, 0.2, 0.1];
    const a = weightedKde(values, w, 0, 6, 50);
    const b = weightedKde(values, w, 0, 6, 50);
    expect(a).toEqual(b);
  });
});

// Deterministic RNG for the tests above.
function mulberry(seed: number): () => number {
  let t = seed;
  return () => {
    t |= 0;
    t = (t + 0x6d2b79f5) | 0;
    let r = Math.imul(t ^ (t >>> 15), 1 | t);
    r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r;
    return ((r ^ (r >>> 14)) >>> 0) / 4294967296;
  };
}

function gaussian(rng: () => number): number {
  // Box-Muller.
  const u1 = Math.max(rng(), 1e-12);
  const u2 = rng();
  return Math.sqrt(-2 * Math.log(u1)) * Math.cos(2 * Math.PI * u2);
}

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

  it("defaults varianceFactor to 2.0 (Beaumont 2009 random-walk recipe)", () => {
    expect(defaultConfig().varianceFactor).toBe(2.0);
  });

  it("defaults probKeepSeed to 0.0 (every accepted particle gets a fresh seed)", () => {
    expect(defaultConfig().probKeepSeed).toBe(0.0);
  });
});

describe("seedObservationHistogram", () => {
  function withSeed(seed: string | undefined): Particle {
    return {
      r0: 0,
      initialInfections: 0,
      weight: 1,
      distance: 0,
      trajectory: [],
      seed,
    };
  }

  it("returns empty arrays when no particles carry a seed", () => {
    const h = seedObservationHistogram([
      withSeed(undefined),
      withSeed(undefined),
    ]);
    expect(h).toEqual({ categories: [], data: [] });
  });

  it("puts every distinct-seed particle in the k=1 bar", () => {
    const h = seedObservationHistogram([
      withSeed("1-1"),
      withSeed("2-2"),
      withSeed("3-3"),
    ]);
    expect(h).toEqual({ categories: ["1"], data: [3] });
  });

  it("counts seeds by their occurrence multiplicity", () => {
    // Two seeds appear once, one seed appears twice, one seed appears
    // three times. Expected histogram:
    //   k=1: 2 seeds
    //   k=2: 1 seed
    //   k=3: 1 seed
    const h = seedObservationHistogram([
      withSeed("a"),
      withSeed("b"),
      withSeed("c"),
      withSeed("c"),
      withSeed("d"),
      withSeed("d"),
      withSeed("d"),
    ]);
    expect(h.categories).toEqual(["1", "2", "3"]);
    expect(h.data).toEqual([2, 1, 1]);
  });

  it("skips undefined-seed particles without distorting counts", () => {
    const h = seedObservationHistogram([
      withSeed("a"),
      withSeed(undefined),
      withSeed("a"),
      withSeed(undefined),
    ]);
    // Only one distinct seed, appearing twice.
    expect(h).toEqual({ categories: ["1", "2"], data: [0, 1] });
  });
});
