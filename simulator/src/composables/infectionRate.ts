// Tagged enum mirroring the Rust `InfectionRate` shape carried over the
// wasm boundary:
//   { type: "constant", value, duration }
//   { type: "empirical", points: [[τ, rate], ...], scale }
//   { type: "library", rates: [[[τ, rate], ...], ...], scale }
//
// For Empirical and Library the point values are **relative hazards** —
// `scale` converts them to absolute rates by multiplication. Defaults
// to 1.0 (the Rust side has `#[serde(default)]` so old JSON without a
// scale field still deserializes).
export type InfectionRate =
  | { type: "constant"; value: number; duration: number }
  | { type: "empirical"; points: [number, number][]; scale: number }
  | { type: "library"; rates: [number, number][][]; scale: number }
  | {
      type: "parametric";
      dist: ParametricDist;
      scale: number;
    };

// A parametric infectiousness shape, mirroring Rust's `ParametricDist`.
// The kernel is the density up to a constant — the model area-normalizes at
// runtime so `scale` reads as the expected R₀ (same as Empirical). The wasm
// side lowers `parametric` to an `empirical` grid before simulating; this
// type exists so the UI/URL/preset can round-trip the compact form.
export type ParametricDist =
  | { dist: "gamma"; shape: number; scale: number }
  | { dist: "lognormal"; mu: number; sigma: number }
  | { dist: "weibull"; shape: number; scale: number };

// Used when switching Constant → Empirical from a UI with no curve yet.
// Values are raw shape — Rust's `normalize::normalize_to_r0` rescales
// at simulation entry so `scale` is the expected R₀ regardless of the
// curve's absolute magnitudes. The point values here are read directly
// in the chart preview, so we keep them human-readable.
export const DEFAULT_EMPIRICAL_POINTS: [number, number][] = [
  [0, 0],
  [2, 1.0],
  [4, 1.2],
  [6, 0.4],
  [8, 0],
];

export const DEFAULT_CONSTANT: InfectionRate = {
  type: "constant",
  value: 0.5,
  duration: 3.0,
};

/// Default magnitude shared by both curve variants. After area
/// normalization (single empirical curve → area = 1; library → mean
/// area = 1), `scale` is the expected R₀ under random mixing.
export const DEFAULT_R0_SCALE = 3.0;

/// Default distribution when switching to the parametric variant: a
/// Gamma(shape 3, scale 1.5) → mean 4.5, most mass within ~12 days.
export const DEFAULT_PARAMETRIC_DIST: ParametricDist = {
  dist: "gamma",
  shape: 3,
  scale: 1.5,
};
/// Grid resolution for sampling the kernel into a preview / lowered curve.
/// Matches Rust's `PARAMETRIC_GRID` so the JS preview tracks the simulated
/// shape.
export const PARAMETRIC_GRID = 128;
/// Fraction of mass captured by the auto-derived support (mirrors Rust's
/// `SUPPORT_MASS_FRACTION`). The parametric variant has no user duration — the
/// support is the distribution's own 99.9% mass point.
const SUPPORT_MASS_FRACTION = 0.999;
/// Trapezoid steps used to locate the support quantile (mirrors Rust).
const SUPPORT_INTEGRATION_STEPS = 8192;

/// Unnormalized density at τ — mirrors Rust's `ParametricDist::kernel`.
/// Zero for τ ≤ 0 and for any non-finite evaluation (the area
/// normalization absorbs the dropped constant + clipped singularities).
export function parametricKernel(d: ParametricDist, t: number): number {
  if (t <= 0) return 0;
  let v: number;
  switch (d.dist) {
    case "gamma":
      v = Math.pow(t, d.shape - 1) * Math.exp(-t / d.scale);
      break;
    case "lognormal": {
      const z = (Math.log(t) - d.mu) / d.sigma;
      v = (1 / t) * Math.exp(-0.5 * z * z);
      break;
    }
    case "weibull":
      v = Math.pow(t, d.shape - 1) * Math.exp(-Math.pow(t / d.scale, d.shape));
      break;
  }
  return Number.isFinite(v) ? v : 0;
}

/// Auto-truncation point: the τ below which `SUPPORT_MASS_FRACTION` of the
/// kernel's mass lies. Mirrors Rust's `ParametricDist::effective_support` so the
/// JS preview lowers over the same support the model simulates (there is no
/// user-specified duration). A geometric scan finds where the tail is
/// negligible, then a trapezoid integral locates the quantile.
export function parametricSupport(d: ParametricDist): number {
  // Upper bound past which the kernel's tail is negligible.
  const START = 1e-3;
  const GROWTH = 1.5;
  const REL = 1e-8;
  const MAX_STEPS = 160;
  let peak = 0;
  let peakT = 0;
  let hi = START;
  for (let i = 0; i < MAX_STEPS; i++) {
    const v = parametricKernel(d, hi);
    if (v > peak) {
      peak = v;
      peakT = hi;
    }
    if (hi > peakT && peak > 0 && v < peak * REL) break;
    hi *= GROWTH;
  }
  // Integrate to get the total, then walk again to the mass-fraction quantile.
  const h = hi / SUPPORT_INTEGRATION_STEPS;
  const total = (() => {
    let cum = 0;
    let prev = parametricKernel(d, 0);
    for (let i = 1; i <= SUPPORT_INTEGRATION_STEPS; i++) {
      const v = parametricKernel(d, h * i);
      cum += 0.5 * (prev + v) * h;
      prev = v;
    }
    return cum;
  })();
  if (!Number.isFinite(total) || total <= 0) return hi;
  const target = SUPPORT_MASS_FRACTION * total;
  let cum = 0;
  let prev = parametricKernel(d, 0);
  for (let i = 1; i <= SUPPORT_INTEGRATION_STEPS; i++) {
    const t = h * i;
    const v = parametricKernel(d, t);
    cum += 0.5 * (prev + v) * h;
    if (cum >= target) return t;
    prev = v;
  }
  return hi;
}

/// Sample the kernel on a grid over `[0, parametricSupport(d)]` — the same
/// lowering the wasm side does, used here for the chart preview.
export function parametricPoints(
  d: ParametricDist,
  n: number = PARAMETRIC_GRID,
): [number, number][] {
  const support = parametricSupport(d);
  const pts: [number, number][] = [];
  for (let i = 0; i < n; i++) {
    const t = (support * i) / (n - 1);
    pts.push([t, parametricKernel(d, t)]);
  }
  return pts;
}

/// Last anchor's time = the deterministic infectious duration for Empirical.
export function empiricalDuration(rate: InfectionRate): number {
  if (rate.type !== "empirical" || rate.points.length === 0) return 0;
  return rate.points[rate.points.length - 1][0];
}

/// Expected number of transmission attempts per index under random
/// mixing. Constant: `value · duration`. Empirical / Library: just
/// `scale` — the wasm side's `normalize::normalize_to_r0` rescales
/// curves so `∫ effective_rate(τ) dτ = scale` regardless of the raw
/// area, so the JS-side computation matches what the model simulates.
function expectedAttempts(rate: InfectionRate): number {
  if (rate.type === "constant") {
    return rate.value * rate.duration;
  }
  if (rate.type === "library") {
    return rate.rates.length ? rate.scale : 0;
  }
  // Parametric (lowered to a unit-area curve) and Empirical both read as
  // `scale` under random mixing.
  if (rate.type === "parametric") {
    return rate.scale;
  }
  return rate.points.length ? rate.scale : 0;
}

/// Expected R₀. With no settings this is just `expectedAttempts(rate)`
/// (the random-mixing R₀). With settings active, transmission is
/// confined to setting members, so we sum a first-generation estimate
/// across settings:
///   R₀ ≈ Σ_s E_n[(n − 1) · (1 − exp(−A · p_s · (n − 1)^(α_s − 1)))]
/// where `A = expectedAttempts(rate)` and `(n − 1)^(α_s − 1)` is the
/// per-target hazard scaling. Treats the index's susceptibles as
/// freshly available (no within-setting chain — that's an epidemic-
/// dynamics question, not an R₀ one).
export function expectedR0(
  rate: InfectionRate,
  settings: import("./settings").SettingType[] = [],
): number {
  const attempts = expectedAttempts(rate);
  if (!settings.length) return attempts;
  let r0 = 0;
  for (const s of settings) {
    for (const [n, q] of s.sizes) {
      if (n <= 1) continue;
      const perTargetRate = attempts * s.proportion * Math.pow(n - 1, s.alpha - 1);
      r0 += q * (n - 1) * (1 - Math.exp(-perTargetRate));
    }
  }
  return r0;
}

/// Switch variant, preserving as much state as we can. Going to constant
/// uses the current curve's duration as the new mean period; going to
/// empirical seeds a default viral-load-shaped curve; going to library
/// expects the caller to seed `rates` itself (it has no good default
/// here — the library payload lives in `virtual:rateLibrary`).
export function withRateType(
  current: InfectionRate,
  next: "constant" | "empirical" | "library" | "parametric",
  defaultLibrary?: [number, number][][],
): InfectionRate {
  if (next === current.type) return current;
  // Best-effort carry-over of the infectious-period length (Constant only —
  // Parametric has no duration, its support is auto-derived).
  const currentDuration =
    empiricalDuration(current) ||
    (current.type === "constant" ? current.duration : 0);
  if (next === "constant") {
    return {
      type: "constant",
      value: 0.5,
      duration: currentDuration || 3.0,
    };
  }
  if (next === "parametric") {
    return {
      type: "parametric",
      dist: { ...DEFAULT_PARAMETRIC_DIST },
      scale: DEFAULT_R0_SCALE,
    };
  }
  // Both curve variants are area-normalized so `scale` reads as the
  // expected R₀ under random mixing (single empirical curve → area = 1;
  // library → mean area = 1). The defaults match: switching between
  // variants preserves the R₀-shaped magnitude.
  if (next === "library") {
    const rates =
      defaultLibrary && defaultLibrary.length
        ? defaultLibrary.map((c) => c.map((p) => [...p] as [number, number]))
        : [
            DEFAULT_EMPIRICAL_POINTS.map((p) => [...p]) as [number, number][],
          ];
    return { type: "library", rates, scale: DEFAULT_R0_SCALE };
  }
  return {
    type: "empirical",
    points: DEFAULT_EMPIRICAL_POINTS.map((p) => [...p]) as [number, number][],
    scale: DEFAULT_R0_SCALE,
  };
}

/// Returns a new Empirical with point `i`'s τ or rate replaced (axis 0
/// = τ, axis 1 = rate), then auto-sorted by τ. Caller must guarantee
/// `rate.type === "empirical"`.
export function withPointUpdated(
  rate: InfectionRate,
  i: number,
  axis: 0 | 1,
  value: number,
): InfectionRate {
  if (rate.type !== "empirical" || !Number.isFinite(value)) return rate;
  const points = rate.points.map(
    (p, idx) =>
      (idx === i
        ? [axis === 0 ? value : p[0], axis === 1 ? value : p[1]]
        : [...p]) as [number, number],
  );
  points.sort((a, b) => a[0] - b[0]);
  return { type: "empirical", points, scale: rate.scale };
}

/// Append a new point 1 time unit past the current last, with the same
/// rate. Auto-sorts.
export function withPointAdded(rate: InfectionRate): InfectionRate {
  if (rate.type !== "empirical") return rate;
  const last = rate.points[rate.points.length - 1];
  const newPoint: [number, number] = last ? [last[0] + 1, last[1]] : [0, 0];
  const points: [number, number][] = [
    ...rate.points.map((p) => [...p] as [number, number]),
    newPoint,
  ];
  points.sort((a, b) => a[0] - b[0]);
  return { type: "empirical", points, scale: rate.scale };
}

/// Remove the point at index `i`. No-op if it would leave fewer than 2
/// points (a segment needs both endpoints).
export function withPointRemoved(rate: InfectionRate, i: number): InfectionRate {
  if (rate.type !== "empirical") return rate;
  if (rate.points.length <= 2) return rate;
  const points = rate.points
    .filter((_, idx) => idx !== i)
    .map((p) => [...p] as [number, number]);
  return { type: "empirical", points, scale: rate.scale };
}

/// Defensive normalizer for InfectionRate values that crossed an
/// external boundary (URL deserialization, preset load, hand-written
/// JSON in the code editor). Older payloads may not include `scale`;
/// the Rust side defaults to 1.0 via `#[serde(default)]`, so we mirror
/// that here so all downstream code can assume `scale` is a finite
/// number.
export function normalizeInfectionRate(rate: InfectionRate): InfectionRate {
  if (rate.type === "constant") return rate;
  const scale = Number.isFinite(rate.scale) ? rate.scale : 1.0;
  if (rate.type === "empirical") {
    return { type: "empirical", points: rate.points, scale };
  }
  if (rate.type === "parametric") {
    return { type: "parametric", dist: rate.dist, scale };
  }
  return { type: "library", rates: rate.rates, scale };
}

/// Parse a CSV string of the shape produced by R's `write.csv` / the
/// bundled library file: header row `id,time,value`, one row per anchor
/// point. Throws on unparseable rows. Rows are grouped by `id` and
/// sorted by `time` within each group. The id order is the natural
/// sort of the ids (numeric if all numeric, otherwise string). The
/// returned curves are raw — the model's `normalize::normalize_to_r0`
/// rescales them at simulation entry so `scale` reads as R₀.
export function parseRateLibraryCsv(text: string): [number, number][][] {
  const lines = text.split(/\r?\n/);
  let header = -1;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line) continue;
    if (/^id\s*,/i.test(line)) {
      header = i;
      break;
    }
    // First non-empty line that doesn't look like a header → assume
    // headerless data.
    header = i - 1;
    break;
  }
  const groups = new Map<string, [number, number][]>();
  for (let i = header + 1; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line) continue;
    const parts = line.split(",").map((p) => p.trim());
    if (parts.length < 3) {
      throw new Error(`row ${i + 1}: expected 3 columns, got ${parts.length}`);
    }
    const [idCol, timeCol, valueCol] = parts;
    const t = Number(timeCol);
    const v = Number(valueCol);
    if (!Number.isFinite(t) || !Number.isFinite(v)) {
      throw new Error(`row ${i + 1}: non-numeric time/value`);
    }
    const arr = groups.get(idCol) ?? [];
    arr.push([t, v]);
    groups.set(idCol, arr);
  }
  if (!groups.size) {
    throw new Error("no data rows found");
  }
  const ids = [...groups.keys()].sort((a, b) => {
    const na = Number(a);
    const nb = Number(b);
    if (Number.isFinite(na) && Number.isFinite(nb)) return na - nb;
    return a.localeCompare(b);
  });
  return ids.map((id) => {
    const pts = groups.get(id)!.slice();
    pts.sort((a, b) => a[0] - b[0]);
    return pts;
  });
}
