//! Intrinsic-infectiousness effect channel.
//!
//! Interventions that reduce a person's **intrinsic** infectiousness λ(τ) —
//! the per-person shedding rate, *not* the settings/contact (total)
//! multiplier — by a relative factor in `[0, 1]` (the fraction *remaining*).
//! Multiple intrinsic modifiers compose **multiplicatively**.
//!
//! Performance: rather than re-aggregate modifiers on every forecast
//! evaluation (the hot path — see `model::evaluate_forecast`), each person
//! caches the running product in a single [`IntrinsicMultiplier`] property.
//! An intervention multiplies its factor in *once*, when it activates; the
//! transmission loop then reads one `f64`. No per-evaluation dynamic
//! dispatch, map lookups, or allocation.
//!
//! Because the cached value only ever stays ≤ 1.0, the forecast upper bound
//! (built from the un-modified λ) remains valid and the existing thinning
//! step accepts at the modified rate for free.
//!
//! Scope: **activate-once** modifiers (a person transmits once over their
//! SIR lifetime, and these only switch *on*). A reversible intrinsic
//! modifier would need its own per-person factor plus a recompute-on-change
//! rather than this multiply-in accumulator.
//!
//! This is the intrinsic channel; the **total/contact** channel is the
//! settings-layer itinerary restriction (`crate::settings`), used by
//! [`crate::modifiers::isolation`]. Both are driven by the shared activation
//! registry in [`crate::modifiers`].

use ixa::prelude::*;
use ixa::{impl_property, Context};

use crate::person::{Person, PersonId};

/// Per-person cached product of every active intrinsic-infectiousness
/// modifier. `1.0` means unmodified. Read once per forecast evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntrinsicMultiplier(pub f64);
impl_property!(
    IntrinsicMultiplier,
    Person,
    default_const = IntrinsicMultiplier(1.0)
);

pub trait IntrinsicModifierExt {
    /// Compose `factor` (relative infectiousness *remaining*, in `[0, 1]`)
    /// into `person`'s cached intrinsic multiplier. Call once per modifier,
    /// at the moment it activates.
    fn apply_intrinsic_modifier(&mut self, person: PersonId, factor: f64);

    /// `person`'s cached intrinsic-infectiousness multiplier (`1.0` when no
    /// modifier is active). Hot-path read in `model::evaluate_forecast`.
    fn intrinsic_multiplier(&self, person: PersonId) -> f64;
}

impl IntrinsicModifierExt for Context {
    fn apply_intrinsic_modifier(&mut self, person: PersonId, factor: f64) {
        debug_assert!(
            (0.0..=1.0).contains(&factor),
            "intrinsic modifier factor must be in [0, 1] — the forecast upper \
             bound assumes modifiers never raise the rate (got {factor})"
        );
        let current = self.get_property::<_, IntrinsicMultiplier>(person).0;
        self.set_property(person, IntrinsicMultiplier(current * factor));
    }

    fn intrinsic_multiplier(&self, person: PersonId) -> f64 {
        self.get_property::<_, IntrinsicMultiplier>(person).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_multiplier_is_one() {
        let mut ctx = Context::new();
        ctx.init_random(0);
        let p = ctx.add_entity(Person).unwrap();
        assert_eq!(ctx.intrinsic_multiplier(p), 1.0);
    }

    #[test]
    fn modifiers_compose_multiplicatively() {
        let mut ctx = Context::new();
        ctx.init_random(0);
        let p = ctx.add_entity(Person).unwrap();
        ctx.apply_intrinsic_modifier(p, 0.5);
        assert!((ctx.intrinsic_multiplier(p) - 0.5).abs() < 1e-12);
        ctx.apply_intrinsic_modifier(p, 0.2);
        // 0.5 × 0.2 = 0.1
        assert!((ctx.intrinsic_multiplier(p) - 0.1).abs() < 1e-12);
    }

    #[test]
    fn modifier_is_per_person() {
        let mut ctx = Context::new();
        ctx.init_random(0);
        let a = ctx.add_entity(Person).unwrap();
        let b = ctx.add_entity(Person).unwrap();
        ctx.apply_intrinsic_modifier(a, 0.3);
        assert!((ctx.intrinsic_multiplier(a) - 0.3).abs() < 1e-12);
        assert_eq!(ctx.intrinsic_multiplier(b), 1.0);
    }
}
