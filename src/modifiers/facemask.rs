//! Facemask intervention: an infected person may put on a mask at a uniformly
//! random time within their infectious period, after which their
//! infectiousness is scaled by `1 - effectiveness` until it ends.
//!
//! Two knobs ([`Facemask`]):
//! * `coverage`: fraction of infected people who ever mask, an independent
//!   `Bernoulli(coverage)` draw each.
//! * `effectiveness`: proportional reduction in infectiousness while masked.
//!   `0` = no effect, `1` = fully blocks shedding.
//!
//! The mask is an ixa `core_epi` infectiousness modifier, so it multiplies
//! with any other modifier on the person and the transmission loop needs to
//! know nothing about it. When it is off no RNG is drawn.

use ixa::core_epi::InfectionManager;
use ixa::prelude::*;
use ixa::{define_rng, Context};
use serde::{Deserialize, Serialize};

use crate::person::PersonId;

define_rng!(FacemaskRng);
ixa::define_modifier!(FacemaskModifier);

/// Facemask intervention configuration.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Facemask {
    /// Fraction of infectious people who mask (`0..=1`).
    pub coverage: f64,
    /// Proportional reduction in intrinsic infectiousness while masked
    /// (`0..=1`). Masked intrinsic rate is multiplied by `1 - effectiveness`.
    pub effectiveness: f64,
}

impl Facemask {
    pub fn validate(&self) -> Result<(), String> {
        if !self.coverage.is_finite() || !(0.0..=1.0).contains(&self.coverage) {
            return Err(format!(
                "facemask coverage must be in [0, 1], got {}",
                self.coverage
            ));
        }
        if !self.effectiveness.is_finite() || !(0.0..=1.0).contains(&self.effectiveness) {
            return Err(format!(
                "facemask effectiveness must be in [0, 1], got {}",
                self.effectiveness
            ));
        }
        Ok(())
    }
}

/// Mask people as they are infected, if enabled.
pub fn register(context: &mut Context, config: Option<Facemask>) {
    if let Some(facemask) = config {
        super::subscribe_to_infections(context, move |context, person| {
            schedule(context, person, facemask);
        });
    }
}

/// Decide whether the newly infected `person` masks and, if so, when.
pub(crate) fn schedule(context: &mut Context, person: PersonId, facemask: Facemask) {
    if !context.sample_bool(FacemaskRng, facemask.coverage) {
        return;
    }
    let infection_time = context
        .infection_time(person)
        .expect("infected person has an infection time");
    let infectious_duration = context
        .infectiousness_duration(person)
        .expect("infected person has a rate function");
    let u: f64 = context.sample_range(FacemaskRng, 0.0..1.0);
    let don_time = infection_time + u * infectious_duration;
    let factor = 1.0 - facemask.effectiveness;
    // Applies only if they are still infectious with this infection then.
    context.schedule_infectiousness_modifier(person, FacemaskModifier, factor, don_time, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::infect_for_test;
    use crate::person::Person;

    #[test]
    fn validate_accepts_unit_interval() {
        Facemask {
            coverage: 0.0,
            effectiveness: 0.0,
        }
        .validate()
        .unwrap();
        Facemask {
            coverage: 1.0,
            effectiveness: 1.0,
        }
        .validate()
        .unwrap();
        Facemask {
            coverage: 0.5,
            effectiveness: 0.7,
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn validate_rejects_out_of_range() {
        assert!(Facemask {
            coverage: -0.1,
            effectiveness: 0.5,
        }
        .validate()
        .is_err());
        assert!(Facemask {
            coverage: 1.1,
            effectiveness: 0.5,
        }
        .validate()
        .is_err());
        assert!(Facemask {
            coverage: 0.5,
            effectiveness: 1.5,
        }
        .validate()
        .is_err());
        assert!(Facemask {
            coverage: 0.5,
            effectiveness: f64::NAN,
        }
        .validate()
        .is_err());
    }

    /// Infect 300 people for `duration`, schedule their masks, and return
    /// each one's infectiousness multiplier at the end of that period.
    fn run_schedule(seed: u64, fm: Facemask, duration: f64) -> Vec<f64> {
        let mut ctx = Context::new();
        ctx.init_random(seed);
        let n = 300;
        let mut people = Vec::with_capacity(n);
        for _ in 0..n {
            let p = ctx.add_entity(Person).unwrap();
            infect_for_test(&mut ctx, p, duration);
            schedule(&mut ctx, p, fm);
            people.push(p);
        }
        ctx.add_plan(duration, |c| c.shutdown());
        ctx.execute();
        people
            .iter()
            .map(|&p| ctx.infectiousness_multiplier(p))
            .collect()
    }

    #[test]
    fn coverage_zero_never_masks() {
        // With coverage 0, no don-mask plan is scheduled, so every person's
        // intrinsic multiplier stays 1.0.
        let mults = run_schedule(
            7,
            Facemask {
                coverage: 0.0,
                effectiveness: 1.0,
            },
            5.0,
        );
        assert!(mults.iter().all(|&m| m == 1.0), "no one should mask");
    }

    #[test]
    fn coverage_one_always_masks_within_window() {
        // With coverage 1 and a deterministic window, everyone dons their
        // mask (within the window), reducing intrinsic infectiousness to
        // 1 - effectiveness.
        let mults = run_schedule(
            3,
            Facemask {
                coverage: 1.0,
                effectiveness: 0.5,
            },
            4.0,
        );
        assert!(
            mults.iter().all(|&m| (m - 0.5).abs() < 1e-12),
            "everyone should mask to 0.5"
        );
    }

    #[test]
    fn partial_coverage_masks_a_fraction() {
        // ~60% of people should mask; the rest stay at 1.0. Loose bounds on
        // a 300-person draw.
        let mults = run_schedule(
            11,
            Facemask {
                coverage: 0.6,
                effectiveness: 1.0,
            },
            5.0,
        );
        let masked = mults.iter().filter(|&&m| m == 0.0).count();
        let unmasked = mults.iter().filter(|&&m| m == 1.0).count();
        assert_eq!(masked + unmasked, mults.len());
        assert!(
            (140..=220).contains(&masked),
            "expected ~180 masked, got {masked}"
        );
    }
}
