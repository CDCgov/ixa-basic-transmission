//! Antiviral treatment: a treated person's infectiousness is scaled by
//! `1 - efficacy` from a fixed `delay` after infection (a proxy for symptom
//! onset and care seeking) until it ends.
//!
//! Three knobs ([`Antiviral`]):
//! * `coverage`: fraction of infections that get treated, an independent
//!   `Bernoulli(coverage)` draw each.
//! * `efficacy`: proportional reduction in infectiousness while treated.
//!   `0` = no effect, `1` = fully blocks shedding.
//! * `delay`: time from infection to treatment start (`>= 0`).
//!
//! Like [`super::facemask`] it is an ixa `core_epi` infectiousness modifier,
//! so a person who is both treated and masked sheds at
//! `(1 - efficacy) * (1 - effectiveness)`. When it is off no RNG is drawn.

use ixa::core_epi::InfectionManager;
use ixa::prelude::*;
use ixa::{define_rng, Context};
use serde::{Deserialize, Serialize};

use crate::person::PersonId;

define_rng!(AntiviralRng);
ixa::define_modifier!(AntiviralModifier);

/// Antiviral treatment configuration.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Antiviral {
    /// Fraction of infections that get treated (`0..=1`).
    pub coverage: f64,
    /// Proportional reduction in intrinsic infectiousness while treated
    /// (`0..=1`). Treated intrinsic rate is multiplied by `1 - efficacy`.
    pub efficacy: f64,
    /// Time from infection to treatment start (`>= 0`).
    pub delay: f64,
}

impl Antiviral {
    pub fn validate(&self) -> Result<(), String> {
        if !self.coverage.is_finite() || !(0.0..=1.0).contains(&self.coverage) {
            return Err(format!(
                "antiviral coverage must be in [0, 1], got {}",
                self.coverage
            ));
        }
        if !self.efficacy.is_finite() || !(0.0..=1.0).contains(&self.efficacy) {
            return Err(format!(
                "antiviral efficacy must be in [0, 1], got {}",
                self.efficacy
            ));
        }
        if !self.delay.is_finite() || self.delay < 0.0 {
            return Err(format!(
                "antiviral delay must be a finite non-negative number, got {}",
                self.delay
            ));
        }
        Ok(())
    }
}

/// Treat people as they are infected, if enabled.
pub fn register(context: &mut Context, config: Option<Antiviral>) {
    if let Some(antiviral) = config {
        super::subscribe_to_infections(context, move |context, person| {
            schedule(context, person, antiviral);
        });
    }
}

/// Decide whether the newly infected `person` is treated and, if so,
/// schedule it.
pub(crate) fn schedule(context: &mut Context, person: PersonId, antiviral: Antiviral) {
    if !context.sample_bool(AntiviralRng, antiviral.coverage) {
        return;
    }
    let infection_time = context
        .infection_time(person)
        .expect("infected person has an infection time");
    let start = infection_time + antiviral.delay;
    let factor = 1.0 - antiviral.efficacy;
    // Applies only if they are still infectious with this infection then.
    context.schedule_infectiousness_modifier(person, AntiviralModifier, factor, start, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::infect_for_test;
    use crate::person::Person;

    #[test]
    fn validate_accepts_valid() {
        Antiviral {
            coverage: 0.5,
            efficacy: 0.7,
            delay: 2.0,
        }
        .validate()
        .unwrap();
        Antiviral {
            coverage: 0.0,
            efficacy: 0.0,
            delay: 0.0,
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn validate_rejects_bad_inputs() {
        assert!(Antiviral {
            coverage: 1.2,
            efficacy: 0.5,
            delay: 1.0,
        }
        .validate()
        .is_err());
        assert!(Antiviral {
            coverage: 0.5,
            efficacy: -0.1,
            delay: 1.0,
        }
        .validate()
        .is_err());
        assert!(Antiviral {
            coverage: 0.5,
            efficacy: 0.5,
            delay: -1.0,
        }
        .validate()
        .is_err());
        assert!(Antiviral {
            coverage: 0.5,
            efficacy: 0.5,
            delay: f64::INFINITY,
        }
        .validate()
        .is_err());
    }

    /// Infect 300 people, schedule their treatment, run until `run_until`,
    /// and return each one's infectiousness multiplier.
    fn run_schedule(seed: u64, av: Antiviral, run_until: f64) -> Vec<f64> {
        let mut ctx = Context::new();
        ctx.init_random(seed);
        let n = 300;
        let mut people = Vec::with_capacity(n);
        for _ in 0..n {
            let p = ctx.add_entity(Person).unwrap();
            infect_for_test(&mut ctx, p, 100.0);
            schedule(&mut ctx, p, av);
            people.push(p);
        }
        ctx.add_plan(run_until, |c| c.shutdown());
        ctx.execute();
        people
            .iter()
            .map(|&p| ctx.infectiousness_multiplier(p))
            .collect()
    }

    #[test]
    fn coverage_zero_never_treats() {
        let mults = run_schedule(
            7,
            Antiviral {
                coverage: 0.0,
                efficacy: 1.0,
                delay: 1.0,
            },
            10.0,
        );
        assert!(mults.iter().all(|&m| m == 1.0), "no one should be treated");
    }

    #[test]
    fn coverage_one_treats_after_delay() {
        // After the delay has elapsed, everyone treated has intrinsic
        // infectiousness reduced to 1 - efficacy.
        let mults = run_schedule(
            3,
            Antiviral {
                coverage: 1.0,
                efficacy: 0.6,
                delay: 2.0,
            },
            10.0,
        );
        assert!(
            mults.iter().all(|&m| (m - 0.4).abs() < 1e-12),
            "everyone treated should be at 0.4"
        );
    }

    #[test]
    fn treatment_not_applied_before_delay() {
        // Shutting down before the delay elapses means no treatment has
        // started yet, so the intrinsic multiplier is still 1.0.
        let mults = run_schedule(
            3,
            Antiviral {
                coverage: 1.0,
                efficacy: 0.6,
                delay: 2.0,
            },
            1.0, // < delay
        );
        assert!(
            mults.iter().all(|&m| m == 1.0),
            "treatment should not have started before the delay"
        );
    }

    #[test]
    fn partial_coverage_treats_a_fraction() {
        // ~60% treated to 0.0 (efficacy 1), rest at 1.0. Loose bounds.
        let mults = run_schedule(
            11,
            Antiviral {
                coverage: 0.6,
                efficacy: 1.0,
                delay: 1.0,
            },
            10.0,
        );
        let treated = mults.iter().filter(|&&m| m == 0.0).count();
        let untreated = mults.iter().filter(|&&m| m == 1.0).count();
        assert_eq!(treated + untreated, mults.len());
        assert!(
            (140..=220).contains(&treated),
            "expected ~180 treated, got {treated}"
        );
    }
}
