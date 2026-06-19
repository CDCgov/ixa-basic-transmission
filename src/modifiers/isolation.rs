//! Isolation intervention: a treated infectious person restricts their
//! contacts to a single setting (their household, say) a fixed `delay` after
//! infection, optionally resuming normal contact after a `duration`.
//!
//! Unlike [`super::facemask`] / [`super::antiviral`] (which scale **intrinsic**
//! infectiousness via the multiply-in cache), isolation is a **structural /
//! total** effect on the contact mixing: it confines the person's *current*
//! itinerary to one setting via the settings-layer override
//! ([`crate::settings::SettingsExt::settings_restrict_to`]). The forecast
//! upper bound (`settings_max_multiplier`) is left unrestricted, so the
//! existing thinning accepts at the reduced rate for free.
//!
//! Models case isolation — only infected individuals, gated here by
//! `coverage` — by restricting the current itinerary to a single setting.
//! Three knobs:
//! * `coverage` — fraction of infections that isolate (`Bernoulli` at infection).
//! * `restrict_to` — the **name** of the setting type to confine to. Resolved
//!   once to a type index at registration; the hot path only sees handles.
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

use super::ModifierExt;
use crate::person::{InfectionStatus, InfectionTime};
use crate::settings::{SettingType, SettingsExt};

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
    let Some(type_index) = settings.iter().position(|s| s.name == iso.restrict_to) else {
        // No matching setting (or none configured) → nothing to restrict to.
        return;
    };
    // A restriction can now occur this run, so the transmission forecast must
    // use the loose `max_s M_s` bound (a restricted person may transmit at a
    // single setting's rate, above their weighted mean). Without this, the
    // forecast uses the tight exact `current_mult` bound — see
    // `SettingsExt::settings_forecast_multiplier`.
    ctx.settings_arm_restriction();
    let Isolation {
        coverage,
        delay,
        duration,
        ..
    } = iso;
    ctx.register_on_infectious(move |c, person, _infectious_duration| {
        // Coverage gate: does this person isolate at all?
        if !c.sample_bool(IsolationRng, coverage) {
            return;
        }
        let t_inf = c.get_property::<_, InfectionTime>(person).0;
        let start = t_inf + delay;
        c.add_plan(start, move |context| {
            // Only isolate the still-infectious; recovered-first → no-op.
            if context.get_property::<_, InfectionStatus>(person) == InfectionStatus::Infectious {
                context.settings_restrict_to(person, type_index);
            }
        });
        if let Some(d) = duration {
            c.add_plan(start + d, move |context| {
                // Resume the full itinerary (harmless no-op if already recovered).
                context.settings_clear_restriction(person);
            });
        }
    });
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
}
