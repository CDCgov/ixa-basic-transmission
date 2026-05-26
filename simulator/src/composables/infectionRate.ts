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
  | { type: "library"; rates: [number, number][][]; scale: number };

// Used when switching Constant → Empirical from a UI with no curve yet.
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

/// Calibration factor for the bundled library curves (from
/// ixa-epi-isolation's production config). The CSV values are relative
/// hazards; multiplied by this they land at a realistic mean R₀ ≈ 3.
export const DEFAULT_LIBRARY_SCALE = 0.05;

/// Trapezoidal area under one piecewise-linear curve.
function curveArea(points: [number, number][]): number {
  let acc = 0;
  for (let i = 0; i < points.length - 1; i++) {
    const [ti, ri] = points[i];
    const [tj, rj] = points[i + 1];
    acc += 0.5 * (ri + rj) * (tj - ti);
  }
  return acc;
}

/// Last anchor's time = the deterministic infectious duration for Empirical.
export function empiricalDuration(rate: InfectionRate): number {
  if (rate.type !== "empirical" || rate.points.length === 0) return 0;
  return rate.points[rate.points.length - 1][0];
}

/// Expected R₀: value · duration for Constant; scale · area-under-curve
/// for Empirical; scale · mean-area-across-curves for Library.
export function expectedR0(rate: InfectionRate): number {
  if (rate.type === "constant") {
    return rate.value * rate.duration;
  }
  if (rate.type === "library") {
    if (!rate.rates.length) return 0;
    const total = rate.rates.reduce((sum, c) => sum + curveArea(c), 0);
    return rate.scale * (total / rate.rates.length);
  }
  return rate.scale * curveArea(rate.points);
}

/// Switch variant, preserving as much state as we can. Going to constant
/// uses the current curve's duration as the new mean period; going to
/// empirical seeds a default viral-load-shaped curve; going to library
/// expects the caller to seed `rates` itself (it has no good default
/// here — the library payload lives in `virtual:rateLibrary`).
export function withRateType(
  current: InfectionRate,
  next: "constant" | "empirical" | "library",
  defaultLibrary?: [number, number][][],
): InfectionRate {
  if (next === current.type) return current;
  if (next === "constant") {
    return {
      type: "constant",
      value: 0.5,
      duration: empiricalDuration(current) || 3.0,
    };
  }
  // Each curve variant has its own "natural" scale because the underlying
  // data differs: the editor's seeded Empirical curve is already at
  // absolute-rate scale (1.0), while the bundled Library curves are
  // relative hazards calibrated against ixa-epi-isolation
  // (DEFAULT_LIBRARY_SCALE = 0.05). Switching variants therefore swaps
  // the scale too — preserving the old one would give the wrong R₀.
  if (next === "library") {
    const rates =
      defaultLibrary && defaultLibrary.length
        ? defaultLibrary.map((c) => c.map((p) => [...p] as [number, number]))
        : [
            DEFAULT_EMPIRICAL_POINTS.map((p) => [...p]) as [number, number][],
          ];
    return { type: "library", rates, scale: DEFAULT_LIBRARY_SCALE };
  }
  return {
    type: "empirical",
    points: DEFAULT_EMPIRICAL_POINTS.map((p) => [...p]) as [number, number][],
    scale: 1.0,
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
  return { type: "library", rates: rate.rates, scale };
}

/// Parse a CSV string of the shape produced by R's `write.csv` / the
/// bundled library file: header row `id,time,value`, one row per anchor
/// point. Throws on unparseable rows. Rows are grouped by `id` and
/// sorted by `time` within each group. The id order is the natural
/// sort of the ids (numeric if all numeric, otherwise string).
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
