//! Transmission modifiers: interventions that act on a person once they are
//! infected. [`register_all`] is the one place that lists them.
//!
//! An intervention changes transmission in one of two ways:
//! * **Infectiousness**: scales the person's own rate with an ixa `core_epi`
//!   infectiousness modifier. Used by [`facemask`] and [`antiviral`].
//! * **Contacts**: changes who the person meets by overriding their settings
//!   itinerary. Used by [`isolation`].
//!
//! Adding a modifier:
//! 1. Give it a module with its config, its own RNG and a `register` that
//!    calls [`subscribe_to_infections`] with what to do for each infected
//!    person.
//! 2. Add its config to [`Modifiers`] and a line to [`register_all`].

pub mod antiviral;
pub mod facemask;
pub mod isolation;

use ixa::core_epi::InfectiousnessAssignedEvent;
use ixa::Context;
use serde::{Deserialize, Serialize};

use crate::model::BasicTransmission;
use crate::modifiers::antiviral::Antiviral;
use crate::modifiers::facemask::Facemask;
use crate::modifiers::isolation::Isolation;
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

/// Run `hook` for each person from now on, once they are infected: their
/// rate function is assigned and the model's `on_infected` has run.
fn subscribe_to_infections(context: &mut Context, hook: impl Fn(&mut Context, PersonId) + 'static) {
    context.subscribe_to_event(
        move |context, event: InfectiousnessAssignedEvent<BasicTransmission>| {
            hook(context, event.person);
        },
    );
}

/// Register every enabled intervention. Call once from `model::setup`, after
/// the settings exist (isolation looks its setting up) and before anyone is
/// infected.
pub fn register_all(context: &mut Context, modifiers: &Modifiers) {
    facemask::register(context, modifiers.facemask);
    antiviral::register(context, modifiers.antiviral);
    isolation::register(context, modifiers.isolation.as_ref());
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::model::infect_for_test;
    use crate::person::Person;
    use ixa::prelude::*;

    #[test]
    fn every_subscriber_runs_for_each_infection() {
        let mut ctx = Context::new();
        ctx.init_random(0);
        let calls = Rc::new(RefCell::new(Vec::new()));
        for hook in [1, 2] {
            let calls = Rc::clone(&calls);
            subscribe_to_infections(&mut ctx, move |_context, person| {
                calls.borrow_mut().push((hook, person));
            });
        }
        let first = ctx.add_entity(Person).unwrap();
        let second = ctx.add_entity(Person).unwrap();
        infect_for_test(&mut ctx, first, 1.0);
        infect_for_test(&mut ctx, second, 1.0);
        ctx.execute();
        assert_eq!(
            *calls.borrow(),
            vec![(1, first), (2, first), (1, second), (2, second)]
        );
    }
}
