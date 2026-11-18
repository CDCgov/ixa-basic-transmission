//! Isolation intervention: a treated infectious person restricts their
//! contacts to a single setting (their household, say) a fixed `delay` after
//! infection, optionally resuming normal contact after a `duration`.
//!
//! Unlike [`super::facemask`] / [`super::antiviral`] (which scale **intrinsic**
//! infectiousness), isolation is a **structural / total** effect on the
//! contact mixing: it gives the person an itinerary of their own that
//! confines their attempts to one setting, at that setting's full
//! multiplier. They stay a member of their other settings, so others can
//! still contact them. Their total changes when isolation starts and ends,
//! so both re-forecast their transmission loop.
//!
//! Models case isolation — only infected individuals, gated here by
//! `coverage` — by restricting the current itinerary to a single setting.
//! Three knobs:
//! * `coverage` — fraction of infections that isolate (`Bernoulli` at infection).
//! * `restrict_to` — the **name** of the setting type to confine to.
//! * `delay` — time from infection to isolation onset.
//! * `duration` — `None` = isolate until recovery; `Some(d)` = a finite
//!   window, then resume the full itinerary.
//!
//! Requires settings to be configured (there's no household to restrict to
//! under global random mixing); with no matching setting it registers
//! nothing. When `Parameters::isolation` is `None` the module is inert.

use ixa::prelude::*;
use ixa::{define_rng, Context};
use serde::{Deserialize, Serialize};

use ixa::core_epi::{InfectionManager, Itinerary, SettingsExt, TransmissionManager};

use super::ModifierExt;
use crate::person::{InfectionStatus, PersonId};
use crate::settings::{Setting, SettingType};

define_rng!(IsolationRng);

/// Isolation intervention configuration. Not `Copy` (`restrict_to` is a
/// `String`); the name is resolved to a type index once at registration so
/// the per-person path never touches it.
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

/// Register the isolation activation hook when enabled. Called from
/// [`super::register_all`] — the single intervention-registration point.
/// `restrict_to` is resolved against `settings` to a type index **once**
/// here; a `None` config or a name that matches no configured setting
/// registers nothing (the latter is a no-op — isolation needs a target).
pub fn register(ctx: &mut Context, config: Option<Isolation>, settings: &[SettingType]) {
    let Some(iso) = config else {
        return;
    };
    if !settings.iter().any(|s| s.name == iso.restrict_to) {
        // No matching setting (or none configured) → nothing to restrict to.
        return;
    }
    let Isolation {
        coverage,
        restrict_to,
        delay,
        duration,
    } = iso;
    ctx.register_on_infectious(move |c, person, _infectious_duration| {
        // Coverage gate: does this person isolate at all?
        if !c.sample_bool(IsolationRng, coverage) {
            return;
        }
        let t_inf = c
            .infection_time(person)
            .expect("infectious person has an infection time");
        let restrict_to = restrict_to.clone();
        c.add_plan(t_inf + delay, move |context| {
            // Only isolate the still-infectious; recovered-first → no-op.
            if context.get_property::<_, InfectionStatus>(person) != InfectionStatus::Infectious {
                return;
            }
            let Some(kind) = context.setting_kind_by_name::<Setting>(&restrict_to) else {
                return;
            };
            if !restrict_itinerary(context, person, kind) {
                return;
            }
            if let Some(d) = duration {
                let end = context.get_current_time() + d;
                context.add_plan(end, move |context| restore_itinerary(context, person));
            }
        });
    });
}

/// Confine `person`'s attempts to their setting of `kind`. Returns whether
/// they have one.
fn restrict_itinerary(context: &mut Context, person: PersonId, kind: Setting) -> bool {
    if context.setting_group(person, kind).is_none() {
        return false;
    }
    context.set_itinerary(person, Itinerary::new([(kind, 1.0)]));
    // Their total may now exceed the forecast their pending proposal was
    // drawn from.
    context.reforecast_transmission(person);
    true
}

/// Give `person` back the itinerary everyone has.
fn restore_itinerary(context: &mut Context, person: PersonId) {
    context.clear_itinerary::<Setting>(person);
    context.reforecast_transmission(person);
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Still a member at work, so others can contact them there.
        assert!(ctx.setting_group(person, work).is_some());

        restore_itinerary(&mut ctx, person);
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
