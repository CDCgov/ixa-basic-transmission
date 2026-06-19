//! Intrinsic-infectiousness modifier registry.
//!
//! Interventions (e.g. [`facemask`]) reduce a person's **intrinsic**
//! infectiousness λ(τ) — the per-person rate, *not* the settings/contact
//! (total) multiplier — by a relative factor in `[0, 1]` (the fraction of
//! infectiousness *remaining*). Multiple modifiers compose
//! **multiplicatively**.
//!
//! Performance: rather than re-aggregate modifiers on every forecast
//! evaluation (the hot path — see `model::evaluate_forecast`), each person
//! caches the running product in a single [`IntrinsicMultiplier`] property.
//! An intervention multiplies its factor in *once*, when it activates; the
//! transmission loop then reads one `f64`. No per-evaluation dynamic
//! dispatch, map lookups, or allocation — and `evaluate_forecast` is
//! intervention-agnostic, so adding a modifier never touches it.
//!
//! Because the cached value only ever stays ≤ 1.0, the forecast upper
//! bound (built from the un-modified λ) remains valid and the existing
//! thinning step accepts at the modified rate for free.
//!
//! Scope: this models **activate-once** modifiers — a person transmits
//! once over their SIR lifetime, and the interventions here only switch
//! *on*. A reversible modifier (mask taken off, isolation ended) would
//! need its own per-person factor plus a recompute-on-change rather than
//! this multiply-in accumulator.
//!
//! Adding a modifier:
//! 1. Give it a module that owns its per-person state + activation timing.
//! 2. Have that activation call [`ModifierExt::apply_intrinsic_modifier`].
//! 3. Register its activation hook in **one place** — [`register_all`] —
//!    via [`ModifierExt::register_on_infectious`]. The transmission loop
//!    runs every registered hook generically (see
//!    [`ModifierExt::run_activation_hooks`]), so the loop itself never
//!    changes when a modifier is added.

pub mod antiviral;
pub mod facemask;

use ixa::prelude::*;
use ixa::{define_data_plugin, impl_property, Context};

use crate::parameters::Parameters;
use crate::person::{Person, PersonId};

/// A hook run once when a person becomes infectious, with that person's
/// realized infectious-period duration. Interventions register one via
/// [`ModifierExt::register_on_infectious`]; the transmission loop runs them
/// all (in registration order) from a single subscriber.
type ActivationHook = Box<dyn Fn(&mut Context, PersonId, f64)>;

/// Registry of intervention activation hooks. Populated once at setup
/// ([`register_all`]); read whenever a person turns infectious.
#[derive(Default)]
struct ModifierActivation {
    on_infectious: Vec<ActivationHook>,
}

define_data_plugin!(ModifierActivationPlugin, ModifierActivation, |_context| {
    ModifierActivation::default()
});

/// Per-person cached product of every active intrinsic-infectiousness
/// modifier. `1.0` means unmodified. Read once per forecast evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntrinsicMultiplier(pub f64);
impl_property!(
    IntrinsicMultiplier,
    Person,
    default_const = IntrinsicMultiplier(1.0)
);

pub trait ModifierExt {
    /// Compose `factor` (relative infectiousness *remaining*, in `[0, 1]`)
    /// into `person`'s cached intrinsic multiplier. Call once per modifier,
    /// at the moment it activates.
    fn apply_intrinsic_modifier(&mut self, person: PersonId, factor: f64);

    /// `person`'s cached intrinsic-infectiousness multiplier (`1.0` when no
    /// modifier is active). Hot-path read in `model::evaluate_forecast`.
    fn intrinsic_multiplier(&self, person: PersonId) -> f64;

    /// Register a hook to run when any person becomes infectious. Called
    /// once per active intervention at setup (see [`register_all`]).
    fn register_on_infectious<F>(&mut self, hook: F)
    where
        F: Fn(&mut Context, PersonId, f64) + 'static;

    /// Run every registered activation hook for `person`, in registration
    /// order, passing their realized infectious-period duration. Called once
    /// from the transmission loop's infectious-event subscriber.
    fn run_activation_hooks(&mut self, person: PersonId, infectious_duration: f64);
}

impl ModifierExt for Context {
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

    fn register_on_infectious<F>(&mut self, hook: F)
    where
        F: Fn(&mut Context, PersonId, f64) + 'static,
    {
        self.get_data_mut(ModifierActivationPlugin)
            .on_infectious
            .push(Box::new(hook));
    }

    fn run_activation_hooks(&mut self, person: PersonId, infectious_duration: f64) {
        // Take the hook list out so we can pass `&mut self` to each hook
        // without a borrow conflict, then put it back (membership is static
        // for the run). Hooks only schedule *future* plans — they never
        // synchronously re-infect anyone — so there is no re-entrancy on
        // this list while it is taken.
        let hooks = std::mem::take(&mut self.get_data_mut(ModifierActivationPlugin).on_infectious);
        for hook in &hooks {
            hook(self, person, infectious_duration);
        }
        self.get_data_mut(ModifierActivationPlugin).on_infectious = hooks;
    }
}

/// Register every enabled intervention's activation hook. Call once from
/// `model::setup`, after the core infectious-event subscriber is installed
/// and before any person is seeded infectious. This is the **single place**
/// that lists the interventions — adding one is a new submodule plus a line
/// here; the transmission loop stays generic.
pub fn register_all(ctx: &mut Context, params: &Parameters) {
    facemask::register(ctx, params.facemask);
    antiviral::register(ctx, params.antiviral);
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
    fn activation_hooks_run_in_order_and_persist() {
        // Hooks registered once are run (in registration order) on every
        // `run_activation_hooks` call and restored afterward.
        let mut ctx = Context::new();
        ctx.init_random(0);
        let p = ctx.add_entity(Person).unwrap();
        ctx.register_on_infectious(|c, person, _dur| c.apply_intrinsic_modifier(person, 0.5));
        ctx.register_on_infectious(|c, person, _dur| c.apply_intrinsic_modifier(person, 0.2));
        ctx.run_activation_hooks(p, 1.0);
        assert!((ctx.intrinsic_multiplier(p) - 0.1).abs() < 1e-12);
        // Hooks restored: a second activation applies them again.
        ctx.run_activation_hooks(p, 1.0);
        assert!((ctx.intrinsic_multiplier(p) - 0.01).abs() < 1e-12);
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
