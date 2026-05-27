// Mirror of the Rust `SettingType` shape (see `src/settings.rs`).
// Sent as-is across the wasm boundary via `Parameters.settings`.
//
// `sizes` is a discrete distribution of `(size, probability)` pairs;
// probabilities should sum to 1 (the Rust side validates).
// `proportion` is the itinerary weight across SettingTypes; selection
// inside the model is `proportion * (n-1)^alpha`, so proportions don't
// have to normalize to 1 exactly.
export type SettingType = {
  name: string;
  alpha: number;
  sizes: [number, number][];
  proportion: number;
};

// US Census Table HH-4 ("Households by Size: 1960 to Present"), 2025
// row, 7+ collapsed to 7. Rounded to 2 dp; sums to 1. Used as the seed
// when the user clicks "+ Add setting" in the editor.
//   https://www.census.gov/data/tables/time-series/demo/families/households.html
export const DEFAULT_SETTING_TYPE: SettingType = {
  name: "household",
  alpha: 1,
  sizes: [
    [1, 0.3],
    [2, 0.34],
    [3, 0.15],
    [4, 0.12],
    [5, 0.06],
    [6, 0.02],
    [7, 0.01],
  ],
  proportion: 1,
};

/// Append a new size row to a SettingType, defaulting to the next
/// integer past the current max and a small probability share.
export function withSizeAdded(s: SettingType): SettingType {
  const last = s.sizes[s.sizes.length - 1];
  const nextSize = last ? last[0] + 1 : 1;
  return {
    ...s,
    sizes: [...s.sizes.map((p) => [...p] as [number, number]), [nextSize, 0]],
  };
}

/// Remove the size row at index `i`. No-op if it would leave fewer
/// than 1 row.
export function withSizeRemoved(s: SettingType, i: number): SettingType {
  if (s.sizes.length <= 1) return s;
  return {
    ...s,
    sizes: s.sizes
      .filter((_, idx) => idx !== i)
      .map((p) => [...p] as [number, number]),
  };
}

/// Replace size row `i`'s size or probability (axis 0 = size, 1 = prob).
export function withSizeUpdated(
  s: SettingType,
  i: number,
  axis: 0 | 1,
  value: number,
): SettingType {
  if (!Number.isFinite(value)) return s;
  const sizes = s.sizes.map(
    (p, idx) =>
      (idx === i
        ? [axis === 0 ? value : p[0], axis === 1 ? value : p[1]]
        : [...p]) as [number, number],
  );
  return { ...s, sizes };
}

/// Sum of size-distribution probabilities. Should be ≈ 1; UI surfaces
/// the deviation so users can renormalize.
export function sizesProbabilitySum(s: SettingType): number {
  return s.sizes.reduce((acc, [, p]) => acc + (Number.isFinite(p) ? p : 0), 0);
}

/// Renormalize the size distribution so probabilities sum to 1. Returns
/// the original if the total is non-positive (nothing to scale).
export function withSizesNormalized(s: SettingType): SettingType {
  const total = sizesProbabilitySum(s);
  if (!(total > 0)) return s;
  return {
    ...s,
    sizes: s.sizes.map(([n, p]) => [n, p / total]),
  };
}

/// Expected per-person setting multiplier `Σ_n P(n) · (n − 1)^α`. This
/// is the per-person mean of `(n − 1)^α` over the size distribution,
/// matching the Rust-side `(n − 1)^α` formula for actual setting size.
export function expectedSettingMultiplier(s: SettingType): number {
  return s.sizes.reduce((acc, [n, p]) => {
    if (!(n > 1)) return acc;
    return acc + p * Math.pow(n - 1, s.alpha);
  }, 0);
}
