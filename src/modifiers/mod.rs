//! Intervention registry — activation machinery shared by all transmission
//! modifiers, plus the single registration point ([`register_all`]).
//!
//! An intervention schedules its effect when a person becomes infectious via
//! [`ModifierExt::register_on_infectious`]; the transmission loop runs every
//! registered hook generically ([`ModifierExt::run_activation_hooks`]), so
//! the loop never changes when one is added. This module is **channel-
//! agnostic** — it only knows *when* effects fire, not *what* they do.
//!
//! Effects land in one of two channels:
//! * **Intrinsic** — scales the per-person shedding rate λ(τ) via ixa
//!   `core_epi`'s keyed infectiousness modifiers. Used by [`facemask`] /
//!   [`antiviral`].
//! * **Total / contact** — restructures contact mixing via the settings-layer
//!   itinerary restriction (`crate::settings`). Used by [`isolation`].
//!
//! Adding a modifier:
//! 1. Give it a module that owns its per-person state + activation timing.
//! 2. Have its activation apply to a channel — `schedule_infectiousness_modifier`
//!    (intrinsic) or a core_epi itinerary change (total).
//! 3. Register its activation hook in **one place** — [`register_all`].

pub mod antiviral;
pub mod facemask;
pub mod isolation;

use ixa::{define_data_plugin, Context};
use serde::{Deserialize, Serialize};

use crate::modifiers::antiviral::Antiviral;
use crate::modifiers::facemask::Facemask;
use crate::modifiers::isolation::Isolation;
use crate::parameters::Parameters;
use crate::person::PersonId;

/// The transmission-modifier set, grouped as one `Parameters` subsection so
/// adding a modifier is a single struct edit (not a field on every
/// `Parameters` literal + arg). Each is `None` (disabled) by default;
/// `#[serde(default)]` lets an omitted field — or the whole `modifiers` key —
/// deserialize as disabled.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Modifiers {
    pub facemask: Option<Facemask>,
    pub antiviral: Option<Antiviral>,
    pub isolation: Option<Isolation>,
}

impl Modifiers {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(facemask) = &self.facemask {
            facemask.validate()?;
        }
        if let Some(antiviral) = &self.antiviral {
            antiviral.validate()?;
        }
        if let Some(isolation) = &self.isolation {
            isolation.validate()?;
        }
        Ok(())
    }
}

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

pub trait ModifierExt {
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
    let m = &params.modifiers;
    // Intrinsic-channel modifiers (scale λ(τ) via core_epi modifiers).
    facemask::register(ctx, m.facemask);
    antiviral::register(ctx, m.antiviral);
    // Total/contact-channel modifier (settings-layer itinerary restriction).
    // Needs the settings list to resolve `restrictTo` to a type index.
    isolation::register(ctx, m.isolation.clone(), &params.settings);
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::person::Person;
    use ixa::prelude::*;

    #[test]
    fn activation_hooks_run_in_order_and_persist() {
        // Hooks registered once are run (in registration order) on every
        // `run_activation_hooks` call and restored afterward.
        let mut ctx = Context::new();
        ctx.init_random(0);
        let p = ctx.add_entity(Person).unwrap();
        let calls = Rc::new(RefCell::new(Vec::new()));

        let first = Rc::clone(&calls);
        ctx.register_on_infectious(move |_c, _person, dur| first.borrow_mut().push((1, dur)));
        let second = Rc::clone(&calls);
        ctx.register_on_infectious(move |_c, _person, dur| second.borrow_mut().push((2, dur)));

        ctx.run_activation_hooks(p, 1.0);
        assert_eq!(*calls.borrow(), vec![(1, 1.0), (2, 1.0)]);

        // Hooks restored: a second activation runs them again.
        ctx.run_activation_hooks(p, 2.0);
        assert_eq!(calls.borrow().len(), 4);
    }
}
