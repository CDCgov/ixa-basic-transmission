// Mirror of the Rust transmission-modifier configs (`src/facemask.rs`,
// `src/antiviral.rs`), sent across the wasm boundary as the optional
// `Parameters.facemask` / `Parameters.antiviral` fields. `null` (the
// default) disables a modifier — the Rust side has `#[serde(default)]`, so
// an omitted or `null` field deserializes to `None`.
//
// Both modifiers reduce an infectious person's *intrinsic* infectiousness
// (the per-person shedding rate, not the contact structure) by a relative
// factor while active; multiple active modifiers compose multiplicatively.

/// Source-control mask: a fraction of infectious people don a mask at a
/// uniform-random time within their infectious period, scaling their
/// intrinsic infectiousness by `1 - effectiveness` thereafter.
export type Facemask = {
  /** Fraction of infectious people who mask, in [0, 1]. */
  coverage: number;
  /** Proportional reduction in intrinsic infectiousness while masked, in [0, 1]. */
  effectiveness: number;
};

/// Antiviral treatment: a fraction of infections start treatment a fixed
/// `delay` after infection, scaling intrinsic infectiousness by
/// `1 - efficacy` thereafter.
export type Antiviral = {
  /** Fraction of infections that get treated, in [0, 1]. */
  coverage: number;
  /** Proportional reduction in intrinsic infectiousness while treated, in [0, 1]. */
  efficacy: number;
  /** Time from infection to treatment start, ≥ 0. */
  delay: number;
};

export const DEFAULT_FACEMASK: Facemask = {
  coverage: 0.5,
  effectiveness: 0.5,
};

export const DEFAULT_ANTIVIRAL: Antiviral = {
  coverage: 0.5,
  efficacy: 0.5,
  delay: 2,
};

/// Fraction of intrinsic infectiousness that *remains* once a modifier is
/// active — the multiplicative factor the model applies (`1 - reduction`),
/// clamped to [0, 1]. Mirrors the Rust `1 - effectiveness` / `1 - efficacy`.
export function remainingInfectiousness(reduction: number): number {
  if (!Number.isFinite(reduction)) return 1;
  return Math.min(1, Math.max(0, 1 - reduction));
}
