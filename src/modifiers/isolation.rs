//! Isolation intervention: an infected person may confine their contacts to
//! one setting (their household, say) from a fixed `delay` after infection,
//! until they recover or for a set `duration`.
//!
//! Unlike [`super::facemask`] and [`super::antiviral`], which scale a
//! person's infectiousness, isolation changes who they meet: it gives them an
//! itinerary of their own with only that setting in it, at that setting's
//! full multiplier. The override also takes them out of their other
//! settings' groups, so nobody contacts them there. Their total changes when
//! isolation starts and ends, so both re-forecast their transmission loop;
//! ending it also re-forecasts the groups they rejoin, whose multipliers rise.
//!
//! Four knobs ([`Isolation`]):
//! * `coverage`: fraction of infections that isolate, an independent
//!   `Bernoulli(coverage)` draw each.
//! * `restrict_to`: the **name** of the setting type to confine to. It must
//!   be one of the configured settings, so isolation needs settings.
//! * `delay`: time from infection to isolation onset.
//! * `duration`: `None` = isolate until recovery; `Some(d)` = for `d`, then
//!   resume the full itinerary.
//!
//! When it is off no RNG is drawn.

use ixa::core_epi::{InfectionManager, Itinerary, SettingsExt, TransmissionManager};
use ixa::prelude::*;
use ixa::{define_rng, Context};
use serde::{Deserialize, Serialize};

use crate::person::{InfectionStatus, PersonId};
use crate::settings::Setting;

define_rng!(IsolationRng);

/// Isolation intervention configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Isolation {
    /// Fraction of infections that isolate (`0..=1`).
    pub coverage: f64,
    /// Name of the setting type to confine contacts to while isolating
    /// (e.g. `"household"`).
    pub restrict_to: String,
    /// Time from infection to isolation onset (`>= 0`).
    pub delay: f64,
    /// `None` = isolate until recovery; `Some(d)` = isolate for `d` then
    /// resume the full itinerary. `d >= 0`.
    #[serde(default)]
    pub duration: Option<f64>,
}

impl Isolation {
    pub fn validate(&self) -> Result<(), String> {
        if !self.coverage.is_finite() || !(0.0..=1.0).contains(&self.coverage) {
            return Err(format!(
                "isolation coverage must be in [0, 1], got {}",
                self.coverage
            ));
        }
        if self.restrict_to.is_empty() {
            return Err("isolation restrictTo must name a setting".to_string());
        }
        if !self.delay.is_finite() || self.delay < 0.0 {
            return Err(format!(
                "isolation delay must be a finite non-negative number, got {}",
                self.delay
            ));
        }
        if let Some(d) = self.duration {
            if !d.is_finite() || d < 0.0 {
                return Err(format!(
                    "isolation duration must be a finite non-negative number, got {d}"
                ));
            }
        }
        Ok(())
    }
}

/// Isolate people as they are infected, if enabled. Call after the settings
/// are set up: the setting to confine to is looked up by name once, here.
pub fn register(context: &mut Context, config: Option<&Isolation>) {
    let Some(isolation) = config else {
        return;
    };
    // Validation rejects a name that is not a configured setting, so the
    // kind is missing only when there were no people to place in settings.
    let Some(kind) = context.setting_kind_by_name::<Setting>(&isolation.restrict_to) else {
        return;
    };
    let Isolation {
        coverage,
        delay,
        duration,
        ..
    } = *isolation;
    super::subscribe_to_infections(context, move |context, person| {
        if !context.sample_bool(IsolationRng, coverage) {
            return;
        }
        let infection_time = context
            .infection_time(person)
            .expect("infected person has an infection time");
        context.add_plan(infection_time + delay, move |context| {
            // Only isolate the still-infectious.
            if context.get_property::<_, InfectionStatus>(person) != InfectionStatus::Infectious {
                return;
            }
            if !restrict_itinerary(context, person, kind) {
                return;
            }
            // Without a duration they isolate until they recover, then go
            // back to their groups like anyone else.
            let end = match duration {
                Some(d) => context.get_current_time() + d,
                None => context
                    .infectiousness_end_time(person)
                    .expect("infectious person has a rate function"),
            };
            context.add_plan(end, move |context| restore_itinerary(context, person));
        });
    });
}

/// Confine `person`'s attempts to their setting of `kind`. Returns whether
/// they have one.
fn restrict_itinerary(context: &mut Context, person: PersonId, kind: Setting) -> bool {
    if context.setting_group(person, kind).is_none() {
        return false;
    }
    context.override_itinerary(person, Itinerary::new([(kind, 1.0)]));
    // Their total may now exceed the forecast their pending proposal was
    // drawn from.
    context.reforecast_transmission(person);
    true
}

/// Give `person` back the itinerary everyone has.
fn restore_itinerary(context: &mut Context, person: PersonId) {
    let kinds = context.setting_kinds::<Setting>();
    let mut left = Vec::new();
    for &kind in &kinds {
        if context.setting_group(person, kind).is_none() {
            left.push(kind);
        }
    }
    context.clear_itinerary_override::<Setting>(person);
    // A group they rejoin grows, raising its members' totals above the
    // forecasts drawn while it was smaller. Only the infectious have a
    // forecast to redraw, the person included.
    let mut stale = vec![person];
    for kind in left {
        if let Some(group) = context.setting_group(person, kind) {
            let members = context.setting_group_members(group);
            stale.extend(members.into_iter().filter(|&member| member != person));
        }
    }
    for member in stale {
        if context.is_infectious(member) {
            context.reforecast_transmission(member);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SettingType;

    fn iso(coverage: f64, delay: f64, duration: Option<f64>) -> Isolation {
        Isolation {
            coverage,
            restrict_to: "household".into(),
            delay,
            duration,
        }
    }

    #[test]
    fn validate_accepts_valid() {
        iso(0.5, 2.0, None).validate().unwrap();
        iso(0.0, 0.0, Some(5.0)).validate().unwrap();
        iso(1.0, 3.0, Some(0.0)).validate().unwrap();
    }

    #[test]
    fn validate_rejects_bad_inputs() {
        assert!(iso(1.2, 2.0, None).validate().is_err());
        assert!(iso(0.5, -1.0, None).validate().is_err());
        assert!(iso(0.5, 2.0, Some(-1.0)).validate().is_err());
        assert!(Isolation {
            coverage: 0.5,
            restrict_to: String::new(),
            delay: 2.0,
            duration: None,
        }
        .validate()
        .is_err());
    }

    fn home_and_work_context() -> (Context, PersonId, Setting, Setting) {
        // home: size 3, alpha 1 → M=2; work: size 5, alpha 1 → M=4.
        let types = [("home", 3), ("work", 5)].map(|(name, size)| SettingType {
            name: name.into(),
            alpha: 1.0,
            sizes: vec![(size, 1.0)],
            proportion: 0.5,
        });
        let mut ctx = Context::new();
        ctx.init_random(7);
        for _ in 0..15 {
            ctx.add_entity(crate::person::Person).unwrap();
        }
        crate::settings::setup(&mut ctx, &types);
        let person = ctx
            .query_result_iterator(crate::person::Person)
            .next()
            .unwrap();
        let kinds = ctx.setting_kinds::<Setting>();
        (ctx, person, kinds[0], kinds[1])
    }

    #[test]
    fn restriction_confines_attempts_to_one_setting_and_restores() {
        let (mut ctx, person, home, work) = home_and_work_context();
        assert!((ctx.current_setting_multiplier::<Setting>(person) - 3.0).abs() < 1e-9);

        // Restricted to home: attempts only there, at its full multiplier.
        assert!(restrict_itinerary(&mut ctx, person, home));
        assert!((ctx.current_setting_multiplier::<Setting>(person) - 2.0).abs() < 1e-9);
        let household = ctx.setting_group(person, home);
        for _ in 0..200 {
            assert_eq!(ctx.sample_setting_by_rate::<Setting>(person), Some(home));
            let contact = ctx
                .sample_setting_contact(person, home)
                .expect("home has other members");
            assert_ne!(contact, person);
            assert_eq!(ctx.setting_group(contact, home), household);
        }
        // Out of their workplace, so nobody contacts them there.
        assert!(ctx.setting_group(person, work).is_none());

        restore_itinerary(&mut ctx, person);
        assert!(ctx.setting_group(person, work).is_some());
        assert!((ctx.current_setting_multiplier::<Setting>(person) - 3.0).abs() < 1e-9);
        let drew_work =
            (0..200).any(|_| ctx.sample_setting_by_rate::<Setting>(person) == Some(work));
        assert!(drew_work, "work is back in the itinerary");
    }

    #[test]
    fn restriction_needs_a_setting_of_that_kind() {
        let (mut ctx, person, _, _) = home_and_work_context();
        let school = ctx.add_setting_kind::<Setting>("school", 1.0);
        assert!(!restrict_itinerary(&mut ctx, person, school));
        assert!((ctx.current_setting_multiplier::<Setting>(person) - 3.0).abs() < 1e-9);
    }
}
