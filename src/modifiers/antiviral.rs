//! Antiviral treatment intervention: a treated infectious person's
//! **intrinsic** infectiousness λ(τ) is scaled by `(1 − efficacy)` starting
//! a fixed `delay` after infection (a proxy for symptom onset + care-
//! seeking), modeling reduced viral shedding under treatment.
//!
//! Three knobs (`Antiviral`):
//! * `coverage` — fraction of infections that get treated. Drawn as an
//!   independent `Bernoulli(coverage)` at infection.
//! * `efficacy` — proportional reduction in intrinsic infectiousness while
//!   treated. `0` = no effect, `1` = fully blocks shedding.
//! * `delay` — time from infection to treatment start (≥ 0).
//!
//! Like [`super::facemask`], this is a consumer of the [`crate::modifiers`]
//! registry: at treatment start it composes its `1 − efficacy` factor into
//! the person's cached intrinsic multiplier. Because modifiers compose
//! multiplicatively, a person who is both treated and masked sheds at
//! `(1 − efficacy) · (1 − mask_effectiveness)`.
//!
//! Contrast with the facemask: treatment fires at a **fixed delay** after
//! infection, a different activation pattern on the same registry (the
//! facemask dons at a uniform time within the infectious window). When
//! `Parameters::antiviral` is `None` the module is inert — no RNG drawn.

use ixa::prelude::*;
use ixa::{define_rng, Context};
use serde::{Deserialize, Serialize};

use super::intrinsic::IntrinsicModifierExt;
use super::ModifierExt;
use crate::person::{InfectionStatus, InfectionTime, PersonId};

define_rng!(AntiviralRng);

/// Antiviral treatment configuration. `Copy` so the model loop can pull it
/// out of `Parameters` by value and avoid holding a borrow across the
/// `&mut self` scheduling call.
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

/// Register the antiviral's activation hook when enabled. Called from
/// [`super::register_all`] — the single intervention-registration point.
/// `None` registers nothing.
pub fn register(ctx: &mut Context, config: Option<Antiviral>) {
    if let Some(av) = config {
        ctx.register_on_infectious(move |c, p, _infectious_duration| {
            c.antiviral_schedule(p, av);
        });
    }
}

pub trait AntiviralExt {
    /// Decide whether `person` is treated and, if so, schedule treatment to
    /// start at `t_inf + delay`. Call once when `person` becomes infectious.
    /// At treatment start the antiviral composes its factor into the
    /// person's intrinsic multiplier (see [`crate::modifiers`]).
    fn antiviral_schedule(&mut self, person: PersonId, antiviral: Antiviral);
}

impl AntiviralExt for Context {
    fn antiviral_schedule(&mut self, person: PersonId, antiviral: Antiviral) {
        // Coverage gate: is this person treated at all?
        if !self.sample_bool(AntiviralRng, antiviral.coverage) {
            return;
        }
        let t_inf = self.get_property::<_, InfectionTime>(person).0;
        let start = t_inf + antiviral.delay;
        let factor = 1.0 - antiviral.efficacy;
        self.add_plan(start, move |context| {
            // Only treat the still-infectious; a person who recovered before
            // treatment would start never gets the modifier.
            if context.get_property::<_, InfectionStatus>(person) == InfectionStatus::Infectious {
                context.apply_intrinsic_modifier(person, factor);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    /// Seed `n` infectious people, schedule treatment, run until `run_until`,
    /// and return each person's resulting intrinsic multiplier.
    fn run_schedule(seed: u64, av: Antiviral, run_until: f64) -> Vec<f64> {
        let mut ctx = Context::new();
        ctx.init_random(seed);
        let n = 300;
        let mut people = Vec::with_capacity(n);
        for _ in 0..n {
            let p = ctx.add_entity(Person).unwrap();
            ctx.set_property(p, InfectionTime(0.0));
            ctx.set_property(p, InfectionStatus::Infectious);
            ctx.antiviral_schedule(p, av);
            people.push(p);
        }
        ctx.add_plan(run_until, |c| c.shutdown());
        ctx.execute();
        people
            .iter()
            .map(|&p| ctx.intrinsic_multiplier(p))
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
