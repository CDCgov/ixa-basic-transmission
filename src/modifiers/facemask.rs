//! Facemask intervention: an infectious person dons a mask once, at a
//! uniformly random time within their infectious period, after which their
//! **intrinsic** infectiousness λ(τ) is scaled by `(1 − effectiveness)` for
//! the rest of their infectious period.
//!
//! Two knobs (`Facemask`):
//! * `coverage` — fraction of infectious people who ever mask. Each
//!   infectious person draws an independent `Bernoulli(coverage)` when they
//!   become infectious.
//! * `effectiveness` — proportional reduction in intrinsic infectiousness
//!   while masked. `0` = no effect, `1` = fully blocks shedding.
//!
//! This is one consumer of the [`crate::modifiers`] registry: at don time
//! it composes its `1 − effectiveness` factor into the person's cached
//! intrinsic multiplier via `apply_intrinsic_modifier`. The registry — not
//! this module — is what the transmission loop reads. Because the factor is
//! ≤ 1, the forecast upper bound stays valid and thinning handles the rest
//! (see `modifiers` + `model::evaluate_forecast`).
//!
//! When `Parameters::facemask` is `None` the module is inert: no RNG is
//! drawn and the intrinsic multiplier stays `1.0`.

use ixa::prelude::*;
use ixa::{define_rng, Context};
use serde::{Deserialize, Serialize};

use super::ModifierExt;
use crate::person::{InfectionStatus, InfectionTime, PersonId};

define_rng!(FacemaskRng);

/// Facemask intervention configuration. `Copy` so the model loop can pull
/// it out of `Parameters` by value and avoid holding a borrow across the
/// `&mut self` scheduling call.
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

/// Register the facemask's activation hook when enabled. Called from
/// [`super::register_all`] — the single intervention-registration point.
/// `None` registers nothing.
pub fn register(ctx: &mut Context, config: Option<Facemask>) {
    if let Some(fm) = config {
        ctx.register_on_infectious(move |c, p, infectious_duration| {
            c.facemask_schedule(p, fm, infectious_duration);
        });
    }
}

pub trait FacemaskExt {
    /// Decide whether `person` masks and, if so, schedule the don-mask plan
    /// at a uniformly random time in `[t_inf, t_inf + infectious_duration]`.
    /// Call once when `person` becomes infectious. `infectious_duration` is
    /// that person's realized infectious period, so the mask lands inside
    /// their actual window. At don time the mask composes its factor into
    /// the person's intrinsic multiplier (see [`crate::modifiers`]).
    fn facemask_schedule(&mut self, person: PersonId, facemask: Facemask, infectious_duration: f64);
}

impl FacemaskExt for Context {
    fn facemask_schedule(
        &mut self,
        person: PersonId,
        facemask: Facemask,
        infectious_duration: f64,
    ) {
        // Coverage gate: does this person mask at all?
        if !self.sample_bool(FacemaskRng, facemask.coverage) {
            return;
        }
        let t_inf = self.get_property::<_, InfectionTime>(person).0;
        let u: f64 = self.sample_range(FacemaskRng, 0.0..1.0);
        let don_time = t_inf + u * infectious_duration;
        let factor = 1.0 - facemask.effectiveness;
        self.add_plan(don_time, move |context| {
            // Only mask the still-infectious; a person who recovered before
            // their (independently drawn, for `Constant`) mask time never dons.
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

    /// Seed an infectious person and run `facemask_schedule`, then execute
    /// any scheduled don-mask plan. Returns the person's resulting intrinsic
    /// multiplier.
    fn run_schedule(seed: u64, fm: Facemask, duration: f64) -> Vec<f64> {
        let mut ctx = Context::new();
        ctx.init_random(seed);
        let n = 300;
        let mut people = Vec::with_capacity(n);
        for _ in 0..n {
            let p = ctx.add_entity(Person).unwrap();
            ctx.set_property(p, InfectionTime(0.0));
            ctx.set_property(p, InfectionStatus::Infectious);
            ctx.facemask_schedule(p, fm, duration);
            people.push(p);
        }
        ctx.add_plan(duration, |c| c.shutdown());
        ctx.execute();
        people
            .iter()
            .map(|&p| ctx.intrinsic_multiplier(p))
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
