// Tagged enum mirroring the Rust `InfectionRate` shape carried over the
// wasm boundary:
//   { type: "constant", value, duration }
//   { type: "empirical", points: [[τ, rate], ...] }    (duration = points.last().τ)
export type InfectionRate =
  | { type: "constant"; value: number; duration: number }
  | { type: "empirical"; points: [number, number][] };

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

/// Last anchor's time = the deterministic infectious duration for Empirical.
export function empiricalDuration(rate: InfectionRate): number {
  if (rate.type !== "empirical" || rate.points.length === 0) return 0;
  return rate.points[rate.points.length - 1][0];
}

/// Expected R₀: value · duration for Constant; trapezoidal area under
/// the curve for Empirical.
export function expectedR0(rate: InfectionRate): number {
  if (rate.type === "constant") {
    return rate.value * rate.duration;
  }
  let acc = 0;
  for (let i = 0; i < rate.points.length - 1; i++) {
    const [ti, ri] = rate.points[i];
    const [tj, rj] = rate.points[i + 1];
    acc += 0.5 * (ri + rj) * (tj - ti);
  }
  return acc;
}

/// Switch variant, preserving as much state as we can. Going to constant
/// uses the current curve's duration as the new mean period; going to
/// empirical seeds a default viral-load-shaped curve.
export function withRateType(
  current: InfectionRate,
  next: "constant" | "empirical",
): InfectionRate {
  if (next === current.type) return current;
  if (next === "constant") {
    return {
      type: "constant",
      value: 0.5,
      duration: empiricalDuration(current) || 3.0,
    };
  }
  return {
    type: "empirical",
    points: DEFAULT_EMPIRICAL_POINTS.map((p) => [...p]) as [number, number][],
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
  return { type: "empirical", points };
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
  return { type: "empirical", points };
}

/// Remove the point at index `i`. No-op if it would leave fewer than 2
/// points (a segment needs both endpoints).
export function withPointRemoved(rate: InfectionRate, i: number): InfectionRate {
  if (rate.type !== "empirical") return rate;
  if (rate.points.length <= 2) return rate;
  const points = rate.points
    .filter((_, idx) => idx !== i)
    .map((p) => [...p] as [number, number]);
  return { type: "empirical", points };
}
