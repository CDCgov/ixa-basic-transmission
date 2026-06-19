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

/// Isolation: a fraction of infections confine their contacts to a single
/// setting (e.g. their household) a fixed `delay` after infection, optionally
/// resuming normal contact after a `duration`. Unlike facemask/antiviral
/// (which scale intrinsic shedding), this is a *contact-structure* change —
/// it requires `settings` to be configured (there's no household to restrict
/// to under global random mixing).
export type Isolation = {
  /** Fraction of infections that isolate, in [0, 1]. */
  coverage: number;
  /** Name of the setting type to confine contacts to (must match a configured setting). */
  restrictTo: string;
  /** Time from infection to isolation onset, ≥ 0. */
  delay: number;
  /** Isolation window: `null` = until recovery; a number = resume contact after it. */
  duration: number | null;
};

export const DEFAULT_ISOLATION: Isolation = {
  coverage: 0.5,
  restrictTo: "",
  delay: 2,
  duration: null,
};

/// Default isolation window length offered when a user switches from
/// "until recovery" to a finite window in the editor.
export const DEFAULT_ISOLATION_DURATION = 5;

/// The transmission-modifier set, grouped as one `Parameters` subsection
/// (mirrors the Rust `Modifiers`). Each modifier is `null` when disabled.
export type Modifiers = {
  facemask: Facemask | null;
  antiviral: Antiviral | null;
  isolation: Isolation | null;
};

export const DEFAULT_MODIFIERS: Modifiers = {
  facemask: null,
  antiviral: null,
  isolation: null,
};

/// Fraction of intrinsic infectiousness that *remains* once a modifier is
/// active — the multiplicative factor the model applies (`1 - reduction`),
/// clamped to [0, 1]. Mirrors the Rust `1 - effectiveness` / `1 - efficacy`.
export function remainingInfectiousness(reduction: number): number {
  if (!Number.isFinite(reduction)) return 1;
  return Math.min(1, Math.max(0, 1 - reduction));
}
