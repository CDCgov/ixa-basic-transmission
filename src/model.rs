use ixa::prelude::*;
use ixa::{define_data_plugin, define_global_property, define_rng, Context};
use rand_distr::Exp;

use crate::modifiers::intrinsic::IntrinsicModifierExt;
use crate::modifiers::ModifierExt;
use crate::parameters::Parameters;
use crate::person::{InfectionStatus, InfectionTime, Person, PersonId};
use crate::rate::{empirical_curve_duration, Curve, EffectiveRate, InfectionRate};
use crate::rate_library::{AssignedRate, Rate, RateLibraryData};
use crate::settings::SettingsExt;
use crate::stats::ModelStats;

define_global_property!(Params, Parameters);

define_rng!(InfectionRng);
define_rng!(RecoveryRng);
define_rng!(RateAssignmentRng);

define_data_plugin!(ModelStatsPlugin, ModelStats, |context| {
    let params = context.get_global_property_value(Params).unwrap();
    ModelStats::new(params.initial_infections)
});

define_data_plugin!(RateLibraryPlugin, RateLibraryData, |_context| {
    RateLibraryData::new()
});

trait InfectionLoop {
    fn get_params(&self) -> &Parameters;
    fn get_stats(&self) -> &ModelStats;
    #[cfg_attr(not(test), allow(dead_code))]
    fn infected_people(&self) -> usize;
    fn forecast_total_infectiousness_multiplier(&self, person: PersonId) -> f64;
    fn current_total_infectiousness_multiplier(&self, person: PersonId) -> f64;
    fn evaluate_forecast(&mut self, infector: PersonId, forecasted: f64) -> bool;
    fn infection_attempt(&mut self, infector: PersonId);
    fn infect_person(&mut self, p: PersonId, t: Option<f64>);
    fn recover_person(&mut self, p: PersonId);
    /// Schedule `p`'s recovery and return the realized infectious-period
    /// duration (`Exp` sample for `Constant`, curve support otherwise) so
    /// the caller can place a facemask within that same window.
    fn schedule_recovery(&mut self, p: PersonId) -> f64;
    fn effective_rate(&self, person: PersonId) -> EffectiveRate<'_>;
    fn schedule_next_infection_attempt(&mut self, infector: PersonId);
    fn setup(&mut self);
}

impl InfectionLoop for Context {
    fn get_params(&self) -> &Parameters {
        self.get_global_property_value(Params).unwrap()
    }
    fn get_stats(&self) -> &ModelStats {
        self.get_data(ModelStatsPlugin)
    }
    fn infected_people(&self) -> usize {
        self.query_entity_count(with!(Person, InfectionStatus::Infectious))
    }
    fn infect_person(&mut self, p: PersonId, t: Option<f64>) {
        if self.get_property::<_, InfectionStatus>(p) != InfectionStatus::Susceptible {
            return;
        }
        // Record infection time before the status flip so the
        // PropertyChangeEvent subscriber can read it on the same person.
        let now = self.get_current_time();
        self.set_property(p, InfectionTime(now));
        self.set_property(p, InfectionStatus::Infectious);
        if let Some(current_t) = t {
            self.get_data_mut(ModelStatsPlugin)
                .record_infection(current_t);
        }
    }
    fn recover_person(&mut self, p: PersonId) {
        self.set_property(p, InfectionStatus::Recovered);
        self.get_data_mut(ModelStatsPlugin).record_recovery();
    }
    fn schedule_recovery(&mut self, p: PersonId) -> f64 {
        // For empirical infectiousness curves, recovery is deterministic at the
        // end of the assigned curve. For `Constant`, recovery follows
        // `Exp(1/duration)` (the `duration` field is the mean).
        let t_inf = self.get_property::<_, InfectionTime>(p).0;
        let recovery_dt = match self.get_params().infection_rate {
            InfectionRate::Empirical { ref points, .. } => empirical_curve_duration(points),
            InfectionRate::Constant { duration, .. } => {
                self.sample_distr(RecoveryRng, Exp::new(1.0 / duration).unwrap())
            }
            InfectionRate::Library { .. } => {
                let rate_id = self.get_property::<_, AssignedRate>(p);
                self.get_data(RateLibraryPlugin).curve(rate_id).duration()
            }
            // Lowered to `Empirical` at `run`/`setup_only` entry; never seen here.
            InfectionRate::Parametric { .. } => {
                unreachable!("Parametric is materialized to Empirical before setup")
            }
        };
        let recovery_time = t_inf + recovery_dt;
        self.add_plan(recovery_time, move |context| {
            if context.get_property::<_, InfectionStatus>(p) == InfectionStatus::Infectious {
                context.recover_person(p);
            }
        });
        recovery_dt
    }
    fn effective_rate(&self, person: PersonId) -> EffectiveRate<'_> {
        // Resolve `InfectionRate` + per-person state into the primitive
        // the inverse-CDF math actually needs
        match &self.get_params().infection_rate {
            InfectionRate::Constant { value, .. } => EffectiveRate::Constant { value: *value },
            InfectionRate::Empirical { points, scale } => EffectiveRate::Empirical {
                curve: self.get_data(RateLibraryPlugin).empirical_or_build(points),
                scale: *scale,
            },
            InfectionRate::Library { scale, .. } => {
                let assigned = self.get_property::<_, AssignedRate>(person);
                let curve = self.get_data(RateLibraryPlugin).curve(assigned);
                EffectiveRate::Empirical {
                    curve,
                    scale: *scale,
                }
            }
            // Lowered to `Empirical` at `run`/`setup_only` entry; never seen here.
            InfectionRate::Parametric { .. } => {
                unreachable!("Parametric is materialized to Empirical before setup")
            }
        }
    }

    fn forecast_total_infectiousness_multiplier(&self, person: PersonId) -> f64 {
        // Upper bound on per-person scaling of the intrinsic rate. The forecast
        // samples on this upper-bound process and `evaluate_forecast` thins down
        // to the true marginal rate. With no itinerary restriction in play this
        // bound is the *exact* constant total rate `Σ p_s · M_s` (so thinning is
        // a no-op); isolation widens it to `max_s M_s`. 1.0 when settings are
        // disabled. See `SettingsExt::settings_forecast_multiplier`.
        self.settings_forecast_multiplier(person)
    }

    fn current_total_infectiousness_multiplier(&self, person: PersonId) -> f64 {
        // True per-person scaling: `Σ p_s · M_s` over the person's settings.
        // Equals 1.0 when settings are disabled.
        self.settings_current_multiplier(person)
    }

    fn infection_attempt(&mut self, infector: PersonId) {
        if let Some(target) = self.settings_sample_contact(infector) {
            let now = self.get_current_time();
            self.infect_person(target, Some(now));
        }
    }

    fn evaluate_forecast(&mut self, infector: PersonId, forecasted: f64) -> bool {
        // Thinning step: the forecast was sampled on the upper-bound rate
        // (intrinsic × forecast_total_infectiousness_multiplier), so the actual
        // current rate is ≤ forecasted. Accept the event with probability
        // current / forecasted; when the two are equal (e.g. settings
        // disabled, where both multipliers are 1.0) the branch short-
        // circuits and consumes no randomness.
        let t_inf = self.get_property::<_, InfectionTime>(infector).0;
        let elapsed = self.get_current_time() - t_inf;
        // Intervention modifiers scale *intrinsic* infectiousness λ(τ) — the
        // per-person rate, not the settings/contact multiplier. Their cached
        // product (`intrinsic_multiplier`, see `crate::modifiers`) is ≤ 1, so
        // the forecast upper bound (built from the un-modified λ) stays valid
        // and this thinning step brings the realized rate down correctly.
        let intrinsic =
            self.effective_rate(infector).rate(elapsed) * self.intrinsic_multiplier(infector);
        let current = intrinsic * self.current_total_infectiousness_multiplier(infector);

        assert!(
            current <= forecasted + 1e-10,
            "person {infector:?}: current rate {current} exceeds forecasted upper bound {forecasted}"
        );

        if current < forecasted {
            self.sample_bool(InfectionRng, current / forecasted)
        } else {
            true
        }
    }

    fn schedule_next_infection_attempt(&mut self, infector: PersonId) {
        // Inverse-CDF sampling on the upper-bound process
        // λ_fc(τ) = λ(τ) · forecast_total_infectiousness_multiplier.
        // Draw e ~ Exp(1) and solve Λ_fc(τ_next) − Λ_fc(elapsed) = e,
        // which rearranges to Λ(τ_next) = Λ(elapsed) + e / fc_mult.
        // `None` from `inverse_cum_rate` means the curve is exhausted —
        // no further attempts for this person.
        let t_inf = self.get_property::<_, InfectionTime>(infector).0;
        let elapsed = self.get_current_time() - t_inf;
        let fc_mult = self.forecast_total_infectiousness_multiplier(infector);

        // A zero upper bound means there are no contacts this person can
        // reach (e.g. they're in a size-1 household and the model is in
        // settings mode). Skip the inverse-CDF — `e / 0 = inf` would
        // otherwise schedule a plan at t = ∞.
        if fc_mult <= 0.0 {
            return;
        }

        let e: f64 = self.sample_distr(InfectionRng, Exp::new(1.0).unwrap());
        let forecast = {
            let eff = self.effective_rate(infector);
            eff.inverse_cum_rate(eff.cum_rate(elapsed) + e / fc_mult)
                .map(|tau| (tau, eff.rate(tau) * fc_mult))
        };
        let Some((elapsed_next, forecasted)) = forecast else {
            return;
        };
        let next_time = t_inf + elapsed_next;
        self.add_plan(next_time, move |context| {
            // The person is no longer infected, exit the loop
            if context.get_property::<_, InfectionStatus>(infector) != InfectionStatus::Infectious {
                return;
            }
            if context.evaluate_forecast(infector, forecasted) {
                context.infection_attempt(infector);
            }
            context.schedule_next_infection_attempt(infector);
        });
    }
    fn setup(&mut self) {
        let &Parameters {
            population,
            initial_infections,
            seed,
            max_time,
            ..
        } = self.get_params();
        self.init_random(seed);
        self.index_property::<Person, InfectionStatus>();

        // When someone becomes infectious, they schedule their own recovery
        // and their own next infection attempt
        self.subscribe_to_event(
            |context, event: PropertyChangeEvent<Person, InfectionStatus>| {
                if event.current != InfectionStatus::Infectious {
                    return;
                }
                let p = event.entity_id;
                let infectious_duration = context.schedule_recovery(p);
                // Run every registered intervention hook generically. Which
                // interventions exist is decided once in
                // `modifiers::register_all`; this loop never changes when
                // one is added (see `crate::modifiers`).
                context.run_activation_hooks(p, infectious_duration);
                context.schedule_next_infection_attempt(p);
            },
        );

        // Register every enabled intervention's activation hook in one place,
        // before anyone is seeded infectious. Clone params once (cheap at
        // setup) to avoid borrowing `self` across the `&mut` registration.
        let params = self.get_params().clone();
        crate::modifiers::register_all(self, &params);

        // For `Library` mode, instantiate one `Rate` entity per curve
        // and populate the EntityMap with precomputed-cum `Curve`s.
        // Cloning the points once at setup is fine — the hot path
        // borrows from the EntityMap and doesn't allocate. (The
        // single-curve `Empirical` variant is lazily built on first
        // access by `RateLibraryData::empirical_or_build`.)
        let library_size: usize =
            if let InfectionRate::Library { rates, .. } = &self.get_params().infection_rate {
                let curves: Vec<Curve> = rates.iter().map(|r| Curve::new(r.clone())).collect();
                let mut ids: Vec<crate::rate_library::RateId> = Vec::with_capacity(curves.len());
                for _ in 0..curves.len() {
                    ids.push(self.add_entity(Rate).unwrap());
                }
                let n = ids.len();
                let lib = self.get_data_mut(RateLibraryPlugin);
                lib.ids = ids.clone();
                lib.curves.reserve(curves.len());
                for (id, curve) in ids.into_iter().zip(curves.into_iter()) {
                    lib.curves.insert(id, curve);
                }
                n
            } else {
                0
            };

        for _ in 0..population {
            let p = self.add_entity(Person).unwrap();
            if library_size > 0 {
                // Uniform random assignment: each person draws independently.
                let idx: usize = self.sample_range(RateAssignmentRng, 0..library_size);
                let assigned = self.get_data(RateLibraryPlugin).ids[idx];
                self.set_property(p, AssignedRate(Some(assigned)));
            }
        }

        // Optional settings-based contact structure. No-op when
        // `params.settings` is empty.
        let settings_types = self.get_params().settings.clone();
        self.settings_setup(&settings_types);

        // Generate and record initial infections
        let sampled: Vec<PersonId> = self.sample_entities(
            InfectionRng,
            with!(Person, InfectionStatus::Susceptible),
            initial_infections,
        );
        for p in sampled {
            self.infect_person(p, None);
        }

        self.add_plan(max_time, |context| {
            context.shutdown();
        });
    }
}

pub fn run(params: Parameters) -> ModelStats {
    params.validate().expect("invalid Parameters");
    // Rescale curve points so `scale` is the expected R₀ under random
    // mixing — the model owns this contract so it holds for every
    // caller (wasm UI, benchmarks, future external embedders).
    let mut params = params;
    // Lower a `Parametric` rate to its `Empirical` curve first, so the rest
    // of the pipeline (normalize, setup, hot path) only ever sees the three
    // concrete variants. `normalize_to_r0` then makes `scale = R₀`.
    crate::rate::materialize_parametric(&mut params.infection_rate);
    crate::normalize::normalize_to_r0(&mut params.infection_rate);
    let mut ctx = Context::new();
    ctx.set_global_property_value(Params, params).unwrap();
    ctx.setup();
    ctx.execute();
    ctx.get_stats().clone()
}

/// Bench-only helper: build a `Context` and run `setup` but **not**
/// `execute`. Splitting this out lets the benchmark harness isolate
/// per-person rate assignment + entity creation from the transmission
/// loop's cost.
#[doc(hidden)]
pub fn setup_only(params: Parameters) {
    params.validate().expect("invalid Parameters");
    let mut params = params;
    // Lower a `Parametric` rate to its `Empirical` curve first, so the rest
    // of the pipeline (normalize, setup, hot path) only ever sees the three
    // concrete variants. `normalize_to_r0` then makes `scale = R₀`.
    crate::rate::materialize_parametric(&mut params.infection_rate);
    crate::normalize::normalize_to_r0(&mut params.infection_rate);
    let mut ctx = Context::new();
    ctx.set_global_property_value(Params, params).unwrap();
    ctx.setup();
    drop(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modifiers::antiviral::{Antiviral, AntiviralExt};
    use crate::modifiers::facemask::{Facemask, FacemaskExt};
    use crate::modifiers::isolation::Isolation;
    use crate::modifiers::Modifiers;
    use crate::settings::SettingType;
    use ixa::assert_almost_eq;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn seeds_initial_infections() {
        let mut ctx = Context::new();
        ctx.set_global_property_value(Params, Parameters::default())
            .unwrap();
        ctx.setup();
        assert_eq!(ctx.infected_people(), 5);
    }

    #[test]
    fn run_returns_nonzero_incidence() {
        let stats = run(Parameters::default());
        assert!(
            stats.cum_incidence() > 0,
            "default params should produce some infections"
        );
    }

    #[test]
    fn parametric_run_matches_lowered_empirical_run() {
        // A Parametric rate is lowered to its Empirical grid at run entry, so
        // running it must be bit-for-bit identical to running the Empirical
        // curve it lowers to (same points, same seed → same RNG stream).
        let para = Parameters {
            infection_rate: InfectionRate::Parametric {
                dist: crate::rate::ParametricDist::Gamma {
                    shape: 3.0,
                    scale: 1.5,
                },
                duration: 14.0,
                scale: 2.0,
            },
            ..Parameters::default()
        };
        let mut lowered = para.clone();
        crate::rate::materialize_parametric(&mut lowered.infection_rate);
        // Sanity: lowering really produced an Empirical curve.
        assert!(matches!(
            lowered.infection_rate,
            InfectionRate::Empirical { .. }
        ));
        let a = run(para);
        let b = run(lowered);
        assert_eq!(a.cum_incidence(), b.cum_incidence());
        assert_eq!(a.timeseries(50.0), b.timeseries(50.0));
    }

    // Kolmogorov-Smirnov statistic against a theoretical CDF. Returns the
    // maximum gap between the empirical and theoretical CDFs.
    #[allow(clippy::cast_precision_loss)]
    fn ks_stat(samples: &mut [f64], theoretical_cdf: impl Fn(f64) -> f64) -> f64 {
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = samples.len() as f64;
        samples
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let lower = i as f64 / n;
                let upper = (i + 1) as f64 / n;
                let f = theoretical_cdf(*x);
                (lower - f).abs().max((upper - f).abs())
            })
            .fold(0.0f64, f64::max)
    }

    // Two-sided KS critical value at α = 0.01 (asymptotic). A truly distributed
    // sample exceeds this with ~1% probability, so it's a principled bound for
    // "the empirical CDF agrees with the theoretical one."
    #[allow(clippy::cast_precision_loss)]
    fn ks_crit_001(n: usize) -> f64 {
        1.63 / (n as f64).sqrt()
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn infection_attempt_times_match_constant_rate_poisson() {
        // One infector + one contact, with the contact reset to Susceptible
        // on each infection so the pool stays constant. The infector is
        // excluded from contact selection, so every attempt hits the contact.
        // Expected:
        //   mean count in [0, T] = rate · T
        //   pooled infection times ~ Uniform(0, T)
        let num_sims: u64 = 20_000;
        let infection_rate = 5.0;
        let max_time = 1.0;

        let times: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
        for seed in 0..num_sims {
            let times_clone = Rc::clone(&times);
            let params = Parameters {
                infection_rate: InfectionRate::Constant {
                    value: infection_rate,
                    // Suppress recovery of the index case during the window.
                    duration: 1e6,
                },
                population: 2,
                initial_infections: 1,
                seed,
                max_time,
                settings: vec![],
                modifiers: Modifiers::default(),
            };
            let mut ctx = Context::new();
            ctx.set_global_property_value(Params, params).unwrap();
            ctx.subscribe_to_event(
                move |context, event: PropertyChangeEvent<Person, InfectionStatus>| {
                    if event.current != InfectionStatus::Infectious {
                        return;
                    }
                    let t = context.get_current_time();
                    if t > 0.0 {
                        times_clone.borrow_mut().push(t);
                        context.set_property(event.entity_id, InfectionStatus::Susceptible);
                    }
                },
            );
            ctx.setup();
            ctx.execute();
        }

        let mut samples = times.borrow().clone();
        let n_samples = samples.len();
        assert!(n_samples > 100, "expected many samples, got {n_samples}");

        let observed_mean = n_samples as f64 / num_sims as f64;
        let expected_mean = infection_rate * max_time;
        assert_almost_eq!(observed_mean, expected_mean, 0.05);

        let ks = ks_stat(&mut samples, |x| {
            if x < 0.0 {
                0.0
            } else if x <= max_time {
                x / max_time
            } else {
                1.0
            }
        });
        let crit = ks_crit_001(n_samples);
        assert!(
            ks < crit,
            "KS {ks:.4} exceeds 1% critical value {crit:.4} for n={n_samples}"
        );
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn infection_attempt_times_match_time_varying_cdf() {
        // Same open-pool setup as the constant-rate test above, but with a
        // linear-ramp schedule λ(t) = a + b·t on [0, T]. The pooled
        // attempt times (conditional on the contact being hit) follow the
        // density λ(t)/Λ(T) on [0, T], so their CDF is Λ(t)/Λ(T) where
        // Λ(t) = a·t + b·t²/2. This is the strongest end-to-end check that
        // inverse-CDF sampling reproduces the right *distribution* of
        // event times, not just the right mean count.
        let num_sims: u64 = 20_000;
        let a = 2.0_f64;
        let b = 6.0_f64;
        let max_time = 1.0_f64;
        let big_lambda = a * max_time + 0.5 * b * max_time * max_time; // Λ(T) = 5.0

        let times: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
        for seed in 0..num_sims {
            let times_clone = Rc::clone(&times);
            // For Empirical, recovery is deterministic at the last anchor
            // time. We make max_time match the curve's last anchor so
            // recovery and shutdown coincide.
            let params = Parameters {
                infection_rate: InfectionRate::Empirical {
                    points: vec![[0.0, a], [max_time, a + b * max_time]],
                    scale: 1.0,
                },
                population: 2,
                initial_infections: 1,
                seed,
                max_time,
                settings: vec![],
                modifiers: Modifiers::default(),
            };
            let mut ctx = Context::new();
            ctx.set_global_property_value(Params, params).unwrap();
            ctx.subscribe_to_event(
                move |context, event: PropertyChangeEvent<Person, InfectionStatus>| {
                    if event.current != InfectionStatus::Infectious {
                        return;
                    }
                    let t = context.get_current_time();
                    if t > 0.0 {
                        times_clone.borrow_mut().push(t);
                        context.set_property(event.entity_id, InfectionStatus::Susceptible);
                    }
                },
            );
            ctx.setup();
            ctx.execute();
        }

        let mut samples = times.borrow().clone();
        let n_samples = samples.len();
        assert!(n_samples > 100, "expected many samples, got {n_samples}");

        // E[count] = Λ(T) = 5.0 per sim (every attempt hits the single contact).
        let observed_mean = n_samples as f64 / num_sims as f64;
        let expected_mean = big_lambda;
        assert_almost_eq!(observed_mean, expected_mean, 0.05);

        let ks = ks_stat(&mut samples, |x| {
            if x < 0.0 {
                0.0
            } else if x <= max_time {
                (a * x + 0.5 * b * x * x) / big_lambda
            } else {
                1.0
            }
        });
        let crit = ks_crit_001(n_samples);
        assert!(
            ks < crit,
            "KS {ks:.4} exceeds 1% critical value {crit:.4} for n={n_samples}"
        );
    }

    #[test]
    fn recovery_times_match_exponential() {
        // All people infected at t=0 with transmission off. Recovery delays
        // should follow Exp(1/infectious_period).
        let population: usize = 5_000;
        let infectious_period = 2.0;
        let max_time = 100.0;

        let recovery_times: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
        let recovery_times_clone = Rc::clone(&recovery_times);

        let params = Parameters {
            infection_rate: InfectionRate::Constant {
                value: 0.0,
                duration: infectious_period,
            },
            population,
            initial_infections: population,
            seed: 0,
            max_time,
            settings: vec![],
            modifiers: Modifiers::default(),
        };
        let mut ctx = Context::new();
        ctx.set_global_property_value(Params, params).unwrap();
        ctx.subscribe_to_event(
            move |context, event: PropertyChangeEvent<Person, InfectionStatus>| {
                if event.current == InfectionStatus::Recovered {
                    recovery_times_clone
                        .borrow_mut()
                        .push(context.get_current_time());
                }
            },
        );
        ctx.setup();
        ctx.execute();

        let mut samples = recovery_times.borrow().clone();
        assert!(
            samples.len() >= population - 5,
            "expected ~{population} recoveries, got {}",
            samples.len()
        );

        let n = samples.len();
        let ks = ks_stat(&mut samples, |x| {
            if x <= 0.0 {
                0.0
            } else {
                1.0 - (-x / infectious_period).exp()
            }
        });
        let crit = ks_crit_001(n);
        assert!(
            ks < crit,
            "KS {ks:.4} exceeds 1% critical value {crit:.4} for n={n}"
        );
    }

    #[test]
    fn zero_rate_no_new_infections() {
        // With zero transmission, no infections occur beyond the seeds.
        let population: usize = 1_000;
        let initial_infections: usize = 50;
        let params = Parameters {
            infection_rate: InfectionRate::Constant {
                value: 0.0,
                duration: 3.0,
            },
            population,
            initial_infections,
            seed: 0,
            max_time: 50.0,
            settings: vec![],
            modifiers: Modifiers::default(),
        };
        let stats = run(params);
        assert_eq!(stats.cum_incidence(), 0);
        let ar = stats.observed_attack_rate(population, initial_infections);
        let expected = initial_infections as f64 / population as f64;
        assert_almost_eq!(ar, expected, 0.0);
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn open_pool_cumulative_incidence_matches_cdf() {
        // One infector + N contacts, no cascade (bypass setup() to skip the
        // global hook), no reset. Each contact faces a constant hazard
        // λ = rate / N (infector excluded) over T, so
        // P(contact infected) = 1 − exp(−λT) and
        // E[cum_incidence] = N · (1 − exp(−λT)).
        let num_sims: u64 = 1_000;
        let population: usize = 6;
        let n_contacts: usize = population - 1;
        let infection_rate: f64 = 3.6;
        let max_time: f64 = 5.0;
        let infectious_period: f64 = 1e6; // suppress recovery in the window

        let lambda = infection_rate / n_contacts as f64;
        let p_infected = 1.0 - f64::exp(-lambda * max_time);
        let expected_cases = n_contacts as f64 * p_infected;

        let mut total: f64 = 0.0;
        for seed in 0..num_sims {
            let params = Parameters {
                infection_rate: InfectionRate::Constant {
                    value: infection_rate,
                    duration: infectious_period,
                },
                population,
                initial_infections: 1,
                seed,
                max_time,
                settings: vec![],
                modifiers: Modifiers::default(),
            };
            let mut ctx = Context::new();
            ctx.set_global_property_value(Params, params).unwrap();
            ctx.init_random(seed);
            let infector = ctx.add_entity(Person).unwrap();
            for _ in 0..n_contacts {
                ctx.add_entity(Person).unwrap();
            }
            ctx.infect_person(infector, None);
            ctx.schedule_next_infection_attempt(infector);
            ctx.add_plan(max_time, |c| c.shutdown());
            ctx.execute();
            total += ctx.get_stats().cum_incidence() as f64;
        }
        let observed = total / num_sims as f64;
        assert_almost_eq!(observed, expected_cases, 0.02);
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn initial_seeding_proportion_matches_params() {
        // Seeding count is deterministic across seeds; the proportion at
        // t=0 should equal initial_infections / population.
        let population: usize = 1_000;
        let initial_infections: usize = 50;
        let num_sims: u64 = 20;

        let mut total_infected: usize = 0;
        for seed in 0..num_sims {
            let params = Parameters {
                infection_rate: InfectionRate::Constant {
                    value: 0.0,
                    duration: 1e6,
                },
                population,
                initial_infections,
                seed,
                max_time: 0.0,
                settings: vec![],
                modifiers: Modifiers::default(),
            };
            let mut ctx = Context::new();
            ctx.set_global_property_value(Params, params).unwrap();
            ctx.setup();
            let count = ctx.infected_people();
            assert_eq!(
                count, initial_infections,
                "seed {seed}: expected {initial_infections} initial infections, got {count}"
            );
            total_infected += count;
        }
        let observed = total_infected as f64 / (population * num_sims as usize) as f64;
        let expected = initial_infections as f64 / population as f64;
        assert_almost_eq!(observed, expected, 0.0);
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn empirical_flat_matches_constant_open_pool() {
        // A 2-point empirical schedule held flat at λ should produce the
        // same mean cumulative incidence as InfectionRate::Constant(λ).
        // Same open-pool setup as `open_pool_cumulative_incidence_matches_cdf`.
        let num_sims: u64 = 1_000;
        let population: usize = 6;
        let n_contacts: usize = population - 1;
        let lambda: f64 = 3.6;
        let max_time: f64 = 5.0;

        let hazard = lambda / n_contacts as f64;
        let p_infected = 1.0 - f64::exp(-hazard * max_time);
        let expected_cases = n_contacts as f64 * p_infected;

        let mut total: f64 = 0.0;
        for seed in 0..num_sims {
            // Empirical recovery is at the last anchor's time; we set it
            // equal to max_time so the index case stays infectious for
            // the whole window.
            let params = Parameters {
                infection_rate: InfectionRate::Empirical {
                    points: vec![[0.0, lambda], [max_time, lambda]],
                    scale: 1.0,
                },
                population,
                initial_infections: 1,
                seed,
                max_time,
                settings: vec![],
                modifiers: Modifiers::default(),
            };
            let mut ctx = Context::new();
            ctx.set_global_property_value(Params, params).unwrap();
            ctx.init_random(seed);
            let infector = ctx.add_entity(Person).unwrap();
            for _ in 0..n_contacts {
                ctx.add_entity(Person).unwrap();
            }
            ctx.infect_person(infector, None);
            ctx.schedule_next_infection_attempt(infector);
            ctx.add_plan(max_time, |c| c.shutdown());
            ctx.execute();
            total += ctx.get_stats().cum_incidence() as f64;
        }
        let observed = total / num_sims as f64;
        assert_almost_eq!(observed, expected_cases, 0.02);
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn empirical_step_schedule_matches_integrated_hazard() {
        // Piecewise-linear schedule that's effectively a step function:
        // λ = lambda_hi on [0, switch], drops to lambda_lo on [switch+ε, T].
        // For a contact in an open pool of `n_contacts` (infector excluded),
        // the survival probability is exp(−∫ λ(t)/n_contacts dt). The integral
        // over the step is
        //     I = lambda_hi/n_contacts · switch
        //       + lambda_lo/n_contacts · (T − switch − ε),
        // ignoring the thin linear ramp at the switch (ε ≪ T).
        let num_sims: u64 = 2_000;
        let population: usize = 6;
        let n_contacts: usize = population - 1;
        let lambda_hi: f64 = 6.0;
        let lambda_lo: f64 = 0.5;
        let switch: f64 = 1.5;
        let eps: f64 = 1e-3;
        let max_time: f64 = 4.0;

        let integrated_hazard =
            (lambda_hi * switch + lambda_lo * (max_time - switch - eps)) / n_contacts as f64;
        let p_infected = 1.0 - f64::exp(-integrated_hazard);
        let expected_cases = n_contacts as f64 * p_infected;

        let mut total: f64 = 0.0;
        for seed in 0..num_sims {
            let params = Parameters {
                infection_rate: InfectionRate::Empirical {
                    points: vec![
                        [0.0, lambda_hi],
                        [switch, lambda_hi],
                        [switch + eps, lambda_lo],
                        [max_time, lambda_lo],
                    ],
                    scale: 1.0,
                },
                population,
                initial_infections: 1,
                seed,
                max_time,
                settings: vec![],
                modifiers: Modifiers::default(),
            };
            let mut ctx = Context::new();
            ctx.set_global_property_value(Params, params).unwrap();
            ctx.init_random(seed);
            let infector = ctx.add_entity(Person).unwrap();
            for _ in 0..n_contacts {
                ctx.add_entity(Person).unwrap();
            }
            ctx.infect_person(infector, None);
            ctx.schedule_next_infection_attempt(infector);
            ctx.add_plan(max_time, |c| c.shutdown());
            ctx.execute();
            total += ctx.get_stats().cum_incidence() as f64;
        }
        let observed = total / num_sims as f64;
        assert_almost_eq!(observed, expected_cases, 0.04);
    }

    #[test]
    fn empirical_chain_transmission_extends_beyond_index_duration() {
        // The index case's infectiousness curve has support [0, 1] only,
        // so the index recovers at wall-clock t = 1. Without per-person
        // clocks (i.e. if τ were wall-clock instead of time-since-infected),
        // no infections could occur past t = 1. Observing infections at
        // wall-clock > 1 conclusively shows each new infectee gets their
        // own [t_inf, t_inf + 1] transmission window.
        let infection_times: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
        let times_clone = Rc::clone(&infection_times);

        let params = Parameters {
            // Recovery is at the last anchor's time (t=1.0).
            infection_rate: InfectionRate::Empirical {
                points: vec![[0.0, 1.5], [1.0, 1.5]],
                scale: 1.0,
            },
            population: 2000,
            initial_infections: 5,
            seed: 42,
            max_time: 10.0,
            settings: vec![],
            modifiers: Modifiers::default(),
        };
        let mut ctx = Context::new();
        ctx.set_global_property_value(Params, params).unwrap();
        ctx.subscribe_to_event(
            move |context, event: PropertyChangeEvent<Person, InfectionStatus>| {
                if event.current == InfectionStatus::Infectious {
                    let t = context.get_current_time();
                    if t > 0.0 {
                        times_clone.borrow_mut().push(t);
                    }
                }
            },
        );
        ctx.setup();
        ctx.execute();

        let times = infection_times.borrow();
        assert!(
            times.len() > 5,
            "expected meaningful chain transmission, got only {} infections",
            times.len()
        );
        let max_t = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            max_t > 1.5,
            "expected infections past t=1.5 (per-person clock); max observed t={max_t}"
        );
    }

    #[test]
    fn library_with_single_curve_matches_empirical() {
        // A library containing one identical curve should produce the
        // same dynamics as the single-Empirical variant, modulo a
        // different RNG stream (RateAssignmentRng draws one sample
        // per person but doesn't alter InfectionRng). We compare mean
        // cumulative incidence across many seeds.
        let curve = vec![[0.0_f64, 1.0], [3.0, 1.0]];

        let mut total_library = 0.0_f64;
        let mut total_empirical = 0.0_f64;
        let n_sims = 100_u64;
        for seed in 0..n_sims {
            let p_lib = Parameters {
                infection_rate: InfectionRate::Library {
                    rates: vec![curve.clone()],
                    scale: 1.0,
                },
                population: 200,
                initial_infections: 3,
                seed,
                max_time: 20.0,
                settings: vec![],
                modifiers: Modifiers::default(),
            };
            let p_emp = Parameters {
                infection_rate: InfectionRate::Empirical {
                    points: curve.clone(),
                    scale: 1.0,
                },
                ..p_lib.clone()
            };
            total_library += run(p_lib).cum_incidence() as f64;
            total_empirical += run(p_emp).cum_incidence() as f64;
        }
        let mean_lib = total_library / n_sims as f64;
        let mean_emp = total_empirical / n_sims as f64;
        // Loose bound: both ensembles see the same dynamics; the only
        // statistical difference is the extra rng consumption for
        // assignment, so they should agree within a few percent.
        let rel = (mean_lib - mean_emp).abs() / mean_emp.max(1.0);
        assert!(
            rel < 0.10,
            "library({mean_lib}) vs empirical({mean_emp}) diverged by {rel}"
        );
    }

    #[test]
    fn library_assigns_a_rate_to_each_person() {
        // After setup, every Person should carry a Some(_) AssignedRate
        // whose value points into the library.
        let library = vec![vec![[0.0, 0.5], [3.0, 0.5]], vec![[0.0, 1.5], [3.0, 1.5]]];
        let params = Parameters {
            infection_rate: InfectionRate::Library {
                rates: library.clone(),
                scale: 1.0,
            },
            population: 50,
            initial_infections: 0,
            seed: 7,
            max_time: 0.0,
            settings: vec![],
            modifiers: Modifiers::default(),
        };
        let mut ctx = Context::new();
        ctx.set_global_property_value(Params, params).unwrap();
        ctx.setup();
        let assigned_count: usize = ctx
            .query_result_iterator(Person)
            .map(|p| ctx.get_property::<_, AssignedRate>(p))
            .filter(|a| a.0.is_some())
            .count();
        assert_eq!(assigned_count, 50);
    }

    #[test]
    fn library_heterogeneous_curves_produce_different_durations() {
        // Two curves of clearly different support: one short (τ=1), one
        // long (τ=10). Run a population where everyone is initially
        // infected, then observe the spread of recovery times. If
        // assignment is per-person, recovery times must straddle both
        // anchor endpoints; if everyone shared a single curve, they'd
        // all bunch near one value.
        let short = vec![[0.0_f64, 0.0], [1.0, 0.0]]; // λ=0 → no transmissions, deterministic recovery at τ=1
        let long = vec![[0.0_f64, 0.0], [10.0, 0.0]]; // recovery at τ=10
        let params = Parameters {
            infection_rate: InfectionRate::Library {
                rates: vec![short.clone(), long.clone()],
                scale: 1.0,
            },
            population: 200,
            initial_infections: 200,
            seed: 11,
            max_time: 50.0,
            settings: vec![],
            modifiers: Modifiers::default(),
        };

        let recovery_times: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
        let clone = Rc::clone(&recovery_times);
        let mut ctx = Context::new();
        ctx.set_global_property_value(Params, params).unwrap();
        ctx.subscribe_to_event(
            move |context, event: PropertyChangeEvent<Person, InfectionStatus>| {
                if event.current == InfectionStatus::Recovered {
                    clone.borrow_mut().push(context.get_current_time());
                }
            },
        );
        ctx.setup();
        ctx.execute();

        let times = recovery_times.borrow().clone();
        let near_short = times.iter().filter(|&&t| (t - 1.0).abs() < 1e-9).count();
        let near_long = times.iter().filter(|&&t| (t - 10.0).abs() < 1e-9).count();
        assert!(
            near_short > 50 && near_long > 50,
            "expected mix of curves; near_short={near_short} near_long={near_long}"
        );
        assert_eq!(
            near_short + near_long,
            times.len(),
            "unexpected recovery time"
        );
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn settings_match_marginal_rate_open_pool() {
        // End-to-end check that the settings forecast + thinning pipeline
        // produces the right marginal rate, mirroring the open-pool tests
        // above (bypass `setup()` to avoid the cascade, no reset).
        //
        // One setting type, every person in one setting of size=population
        // with alpha=1 → M = pop - 1. proportion = 1, so current = max = M.
        // Per-contact constant hazard:
        //   λ = base_rate · M / n_contacts  (n_contacts = pop - 1 = M, so λ = base_rate).
        let num_sims: u64 = 1_000;
        let population: usize = 6;
        let n_contacts: usize = population - 1;
        let base_rate: f64 = 0.7;
        let max_time: f64 = 4.0;

        let lambda = base_rate;
        let p_infected = 1.0 - f64::exp(-lambda * max_time);
        let expected_cases = n_contacts as f64 * p_infected;

        let setting_type = crate::settings::SettingType {
            name: "all".into(),
            alpha: 1.0,
            sizes: vec![(population, 1.0)],
            proportion: 1.0,
        };

        let mut total: f64 = 0.0;
        for seed in 0..num_sims {
            let params = Parameters {
                infection_rate: InfectionRate::Constant {
                    value: base_rate,
                    duration: 1e6, // suppress recovery in the window
                },
                population,
                initial_infections: 1,
                seed,
                max_time,
                settings: vec![setting_type.clone()],
                modifiers: Modifiers::default(),
            };
            let mut ctx = Context::new();
            ctx.set_global_property_value(Params, params).unwrap();
            ctx.init_random(seed);
            let infector = ctx.add_entity(Person).unwrap();
            for _ in 0..n_contacts {
                ctx.add_entity(Person).unwrap();
            }
            ctx.settings_setup(&[setting_type.clone()]);
            ctx.infect_person(infector, None);
            ctx.schedule_next_infection_attempt(infector);
            ctx.add_plan(max_time, |c| c.shutdown());
            ctx.execute();
            total += ctx.get_stats().cum_incidence() as f64;
        }
        let observed = total / num_sims as f64;
        assert_almost_eq!(observed, expected_cases, 0.03);
    }

    #[test]
    fn facemask_coverage_zero_matches_no_facemask() {
        // coverage=0 schedules no masks. The facemask RNG stream is
        // independent of the transmission streams, so a coverage-0 run must
        // be bit-identical to the no-facemask run.
        let base = Parameters {
            infection_rate: InfectionRate::Constant {
                value: 0.5,
                duration: 3.0,
            },
            population: 2000,
            initial_infections: 5,
            seed: 1,
            max_time: 60.0,
            settings: vec![],
            modifiers: Modifiers::default(),
        };
        let no_mask = run(base.clone());
        let cov0 = run(Parameters {
            modifiers: Modifiers {
                facemask: Some(Facemask {
                    coverage: 0.0,
                    effectiveness: 1.0,
                }),
                ..Default::default()
            },
            ..base
        });
        assert_eq!(no_mask.cum_incidence(), cov0.cum_incidence());
    }

    #[test]
    fn facemask_zero_effectiveness_is_noop() {
        // Masks are donned (coverage=1) but a zero-effectiveness mask leaves
        // intrinsic infectiousness unchanged, so dynamics match no-facemask.
        let base = Parameters {
            infection_rate: InfectionRate::Constant {
                value: 0.5,
                duration: 3.0,
            },
            population: 2000,
            initial_infections: 5,
            seed: 2,
            max_time: 60.0,
            settings: vec![],
            modifiers: Modifiers::default(),
        };
        let no_mask = run(base.clone());
        let eff0 = run(Parameters {
            modifiers: Modifiers {
                facemask: Some(Facemask {
                    coverage: 1.0,
                    effectiveness: 0.0,
                }),
                ..Default::default()
            },
            ..base
        });
        assert_eq!(no_mask.cum_incidence(), eff0.cum_incidence());
    }

    #[test]
    fn facemask_full_coverage_reduces_epidemic_incidence() {
        // A fully-blocking mask worn by everyone must strictly cut the
        // realized epidemic size relative to no intervention.
        let base = Parameters {
            infection_rate: InfectionRate::Constant {
                value: 0.5,
                duration: 3.0,
            },
            population: 5000,
            initial_infections: 10,
            seed: 3,
            max_time: 80.0,
            settings: vec![],
            modifiers: Modifiers::default(),
        };
        let no_mask = run(base.clone());
        let masked = run(Parameters {
            modifiers: Modifiers {
                facemask: Some(Facemask {
                    coverage: 1.0,
                    effectiveness: 1.0,
                }),
                ..Default::default()
            },
            ..base
        });
        assert!(
            masked.cum_incidence() < no_mask.cum_incidence(),
            "masked incidence ({}) should be below unmasked ({})",
            masked.cum_incidence(),
            no_mask.cum_incidence()
        );
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn facemask_full_block_open_pool_matches_masked_cdf() {
        // One infector + N contacts, no cascade (same open-pool harness as
        // `open_pool_cumulative_incidence_matches_cdf`). The infector dons a
        // fully-blocking mask (effectiveness=1) at a uniform time m ~ U(0, D)
        // in its deterministic infectious window [0, D]; after masking it
        // transmits at 0, so a contact accumulates hazard (λ/n)·m. Over the
        // open pool (infector excluded, per-contact hazard λ/n), with m
        // marginalized out:
        //   E[P(infected)] = 1 − (n / (λD)) · (1 − e^{−λD/n})
        //   E[cases]       = n · E[P]
        let num_sims: u64 = 10_000;
        let population: usize = 6;
        let n_contacts = population - 1;
        let lambda = 3.6_f64;
        let d = 5.0_f64; // empirical curve support == infectious duration
        let facemask = Facemask {
            coverage: 1.0,
            effectiveness: 1.0,
        };

        let r = lambda * d / n_contacts as f64;
        let expected_p = 1.0 - (n_contacts as f64 / (lambda * d)) * (1.0 - (-r).exp());
        let expected_cases = n_contacts as f64 * expected_p;

        let mut total = 0.0_f64;
        for seed in 0..num_sims {
            let params = Parameters {
                infection_rate: InfectionRate::Empirical {
                    points: vec![[0.0, lambda], [d, lambda]],
                    scale: 1.0,
                },
                population,
                initial_infections: 1,
                seed,
                max_time: d,
                settings: vec![],
                modifiers: Modifiers {
                    facemask: Some(facemask),
                    ..Default::default()
                },
            };
            let mut ctx = Context::new();
            ctx.set_global_property_value(Params, params).unwrap();
            ctx.init_random(seed);
            let infector = ctx.add_entity(Person).unwrap();
            for _ in 0..n_contacts {
                ctx.add_entity(Person).unwrap();
            }
            ctx.infect_person(infector, None);
            // Mirror the setup() subscriber for the index case only.
            let dt = ctx.schedule_recovery(infector);
            ctx.facemask_schedule(infector, facemask, dt);
            ctx.schedule_next_infection_attempt(infector);
            ctx.add_plan(d, |c| c.shutdown());
            ctx.execute();
            total += ctx.get_stats().cum_incidence() as f64;
        }
        let observed = total / num_sims as f64;
        assert_almost_eq!(observed, expected_cases, 0.05);
    }

    #[test]
    fn antiviral_coverage_zero_matches_no_antiviral() {
        // coverage=0 schedules no treatment; the antiviral RNG stream is
        // independent of the transmission streams, so results are identical
        // to the no-antiviral run.
        let base = Parameters {
            infection_rate: InfectionRate::Constant {
                value: 0.5,
                duration: 3.0,
            },
            population: 2000,
            initial_infections: 5,
            seed: 1,
            max_time: 60.0,
            settings: vec![],
            modifiers: Modifiers::default(),
        };
        let no_tx = run(base.clone());
        let cov0 = run(Parameters {
            modifiers: Modifiers {
                antiviral: Some(Antiviral {
                    coverage: 0.0,
                    efficacy: 1.0,
                    delay: 1.0,
                }),
                ..Default::default()
            },
            ..base
        });
        assert_eq!(no_tx.cum_incidence(), cov0.cum_incidence());
    }

    #[test]
    fn antiviral_zero_efficacy_is_noop() {
        // Treatment is scheduled (coverage=1) but a zero-efficacy antiviral
        // leaves intrinsic infectiousness unchanged, so dynamics match
        // no-antiviral.
        let base = Parameters {
            infection_rate: InfectionRate::Constant {
                value: 0.5,
                duration: 3.0,
            },
            population: 2000,
            initial_infections: 5,
            seed: 2,
            max_time: 60.0,
            settings: vec![],
            modifiers: Modifiers::default(),
        };
        let no_tx = run(base.clone());
        let eff0 = run(Parameters {
            modifiers: Modifiers {
                antiviral: Some(Antiviral {
                    coverage: 1.0,
                    efficacy: 0.0,
                    delay: 1.0,
                }),
                ..Default::default()
            },
            ..base
        });
        assert_eq!(no_tx.cum_incidence(), eff0.cum_incidence());
    }

    #[test]
    fn antiviral_reduces_epidemic_incidence() {
        // Full-coverage, fully-blocking treatment with a short delay must
        // strictly cut the realized epidemic size.
        let base = Parameters {
            infection_rate: InfectionRate::Constant {
                value: 0.5,
                duration: 3.0,
            },
            population: 5000,
            initial_infections: 10,
            seed: 3,
            max_time: 80.0,
            settings: vec![],
            modifiers: Modifiers::default(),
        };
        let no_tx = run(base.clone());
        let treated = run(Parameters {
            modifiers: Modifiers {
                antiviral: Some(Antiviral {
                    coverage: 1.0,
                    efficacy: 1.0,
                    delay: 1.0,
                }),
                ..Default::default()
            },
            ..base
        });
        assert!(
            treated.cum_incidence() < no_tx.cum_incidence(),
            "treated incidence ({}) should be below untreated ({})",
            treated.cum_incidence(),
            no_tx.cum_incidence()
        );
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn antiviral_full_block_open_pool_matches_treated_cdf() {
        // Open-pool harness (cf. the facemask analytical test). A treated
        // infector (efficacy=1) transmits at full λ until treatment starts at
        // the fixed `delay`, then 0. The mask time is random; the treatment
        // time is *deterministic*, so a contact accumulates exactly hazard
        // (λ/n)·delay (for delay < D), with no marginalization:
        //   E[cases] = n · (1 − e^{−(λ/n)·delay})
        let num_sims: u64 = 5000;
        let population: usize = 6;
        let n_contacts = population - 1;
        let lambda = 3.6_f64;
        let d = 5.0_f64; // infectious window
        let delay = 2.0_f64; // treatment start (< D)
        let antiviral = Antiviral {
            coverage: 1.0,
            efficacy: 1.0,
            delay,
        };

        let integrated = lambda / n_contacts as f64 * delay;
        let expected_cases = n_contacts as f64 * (1.0 - (-integrated).exp());

        let mut total = 0.0_f64;
        for seed in 0..num_sims {
            let params = Parameters {
                infection_rate: InfectionRate::Empirical {
                    points: vec![[0.0, lambda], [d, lambda]],
                    scale: 1.0,
                },
                population,
                initial_infections: 1,
                seed,
                max_time: d,
                settings: vec![],
                modifiers: Modifiers {
                    antiviral: Some(antiviral),
                    ..Default::default()
                },
            };
            let mut ctx = Context::new();
            ctx.set_global_property_value(Params, params).unwrap();
            ctx.init_random(seed);
            let infector = ctx.add_entity(Person).unwrap();
            for _ in 0..n_contacts {
                ctx.add_entity(Person).unwrap();
            }
            ctx.infect_person(infector, None);
            // Mirror the setup() subscriber for the index case only.
            ctx.schedule_recovery(infector);
            ctx.antiviral_schedule(infector, antiviral);
            ctx.schedule_next_infection_attempt(infector);
            ctx.add_plan(d, |c| c.shutdown());
            ctx.execute();
            total += ctx.get_stats().cum_incidence() as f64;
        }
        let observed = total / num_sims as f64;
        assert_almost_eq!(observed, expected_cases, 0.05);
    }

    #[test]
    fn facemask_and_antiviral_compose() {
        // Two intrinsic modifiers stack: combining a mask and an antiviral
        // cuts incidence at least as much as either alone, and strictly
        // below no intervention.
        let base = Parameters {
            infection_rate: InfectionRate::Constant {
                value: 0.5,
                duration: 3.0,
            },
            population: 5000,
            initial_infections: 10,
            seed: 5,
            max_time: 80.0,
            settings: vec![],
            modifiers: Modifiers::default(),
        };
        let mask = Some(Facemask {
            coverage: 1.0,
            effectiveness: 0.6,
        });
        let av = Some(Antiviral {
            coverage: 1.0,
            efficacy: 0.6,
            delay: 1.0,
        });

        let none = run(base.clone()).cum_incidence();
        let mask_only = run(Parameters {
            modifiers: Modifiers {
                facemask: mask,
                ..Default::default()
            },
            ..base.clone()
        })
        .cum_incidence();
        let av_only = run(Parameters {
            modifiers: Modifiers {
                antiviral: av,
                ..Default::default()
            },
            ..base.clone()
        })
        .cum_incidence();
        let both = run(Parameters {
            modifiers: Modifiers {
                facemask: mask,
                antiviral: av,
                ..Default::default()
            },
            ..base
        })
        .cum_incidence();

        assert!(both < none, "both ({both}) should be below none ({none})");
        assert!(
            both <= mask_only && both <= av_only,
            "both ({both}) should not exceed either single intervention \
             (mask {mask_only}, antiviral {av_only})"
        );
    }

    // --- Isolation (itinerary restriction, total/contact channel) ---------

    fn household_and_work() -> Vec<SettingType> {
        // Small household + large workplace. Isolating to household cuts the
        // (much larger) workplace contacts.
        vec![
            SettingType {
                name: "household".into(),
                alpha: 1.0,
                sizes: vec![(3, 1.0)],
                proportion: 0.5,
            },
            SettingType {
                name: "work".into(),
                alpha: 1.0,
                sizes: vec![(20, 1.0)],
                proportion: 0.5,
            },
        ]
    }

    fn isolation_base(seed: u64) -> Parameters {
        Parameters {
            infection_rate: InfectionRate::Constant {
                value: 0.5,
                duration: 3.0,
            },
            population: 5000,
            initial_infections: 10,
            seed,
            max_time: 80.0,
            settings: household_and_work(),
            modifiers: Modifiers::default(),
        }
    }

    #[test]
    fn isolation_coverage_zero_matches_no_isolation() {
        // coverage=0 sets no restriction; `IsolationRng` is independent, so the
        // run is bit-identical to no-isolation.
        let base = isolation_base(1);
        let no_iso = run(base.clone());
        let cov0 = run(Parameters {
            modifiers: Modifiers {
                isolation: Some(Isolation {
                    coverage: 0.0,
                    restrict_to: "household".into(),
                    delay: 1.0,
                    duration: None,
                }),
                ..Default::default()
            },
            ..base
        });
        assert_eq!(no_iso.cum_incidence(), cov0.cum_incidence());
    }

    #[test]
    fn isolation_unknown_setting_is_noop() {
        // A restrictTo that matches no configured setting registers nothing,
        // so the run matches no-isolation exactly.
        let base = isolation_base(2);
        let no_iso = run(base.clone());
        let bad = run(Parameters {
            modifiers: Modifiers {
                isolation: Some(Isolation {
                    coverage: 1.0,
                    restrict_to: "spaceship".into(),
                    delay: 0.0,
                    duration: None,
                }),
                ..Default::default()
            },
            ..base
        });
        assert_eq!(no_iso.cum_incidence(), bad.cum_incidence());
    }

    #[test]
    fn isolation_reduces_epidemic_incidence() {
        // Full-coverage isolation to the small household (M=2) instead of the
        // large workplace (M=19) must strictly cut the epidemic.
        let base = isolation_base(3);
        let no_iso = run(base.clone());
        let isolated = run(Parameters {
            modifiers: Modifiers {
                isolation: Some(Isolation {
                    coverage: 1.0,
                    restrict_to: "household".into(),
                    delay: 0.0,
                    duration: None,
                }),
                ..Default::default()
            },
            ..base
        });
        assert!(
            isolated.cum_incidence() < no_iso.cum_incidence(),
            "isolated incidence ({}) should be below un-isolated ({})",
            isolated.cum_incidence(),
            no_iso.cum_incidence()
        );
    }

    #[test]
    fn finite_duration_isolation_allows_more_transmission_than_persistent() {
        // A finite isolation window resumes full contact afterward, so it
        // permits more transmission than isolating until recovery.
        let base = isolation_base(4);
        let persistent = run(Parameters {
            modifiers: Modifiers {
                isolation: Some(Isolation {
                    coverage: 1.0,
                    restrict_to: "household".into(),
                    delay: 0.0,
                    duration: None,
                }),
                ..Default::default()
            },
            ..base.clone()
        });
        let finite = run(Parameters {
            modifiers: Modifiers {
                isolation: Some(Isolation {
                    coverage: 1.0,
                    restrict_to: "household".into(),
                    delay: 0.0,
                    duration: Some(2.0),
                }),
                ..Default::default()
            },
            ..base
        });
        assert!(
            finite.cum_incidence() > persistent.cum_incidence(),
            "finite-window isolation ({}) should permit more than persistent ({})",
            finite.cum_incidence(),
            persistent.cum_incidence()
        );
    }

    #[test]
    fn isolation_and_facemask_compose() {
        // Total (isolation) and intrinsic (facemask) channels stack: together
        // at least as low as either alone, and below no intervention.
        let base = isolation_base(5);
        let iso = Some(Isolation {
            coverage: 1.0,
            restrict_to: "household".into(),
            delay: 0.0,
            duration: None,
        });
        let mask = Some(Facemask {
            coverage: 1.0,
            effectiveness: 0.6,
        });

        let none = run(base.clone()).cum_incidence();
        let iso_only = run(Parameters {
            modifiers: Modifiers {
                isolation: iso.clone(),
                ..Default::default()
            },
            ..base.clone()
        })
        .cum_incidence();
        let mask_only = run(Parameters {
            modifiers: Modifiers {
                facemask: mask,
                ..Default::default()
            },
            ..base.clone()
        })
        .cum_incidence();
        let both = run(Parameters {
            modifiers: Modifiers {
                isolation: iso,
                facemask: mask,
                ..Default::default()
            },
            ..base
        })
        .cum_incidence();

        assert!(both < none, "both ({both}) should be below none ({none})");
        assert!(
            both <= iso_only && both <= mask_only,
            "both ({both}) should not exceed either alone (iso {iso_only}, mask {mask_only})"
        );
    }

    #[test]
    fn isolation_to_peak_setting_keeps_forecast_bound_valid() {
        // Restricting to the *highest*-contact setting (work, M=19) raises an
        // isolating person's current multiplier above their weighted mean
        // (current_mult = 0.5·2 + 0.5·19 = 10.5). This is exactly the case the
        // loose forecast bound exists for: `isolation::register` arms
        // restriction, so the forecast uses max_s M_s (= 19) and the
        // `current ≤ forecasted` invariant in `evaluate_forecast` holds. Were
        // arming dropped (tight bound 10.5), this run would trip that assert —
        // unlike the household-restriction tests, which only restrict *down*.
        let base = isolation_base(6);
        let to_work = run(Parameters {
            modifiers: Modifiers {
                isolation: Some(Isolation {
                    coverage: 1.0,
                    restrict_to: "work".into(),
                    delay: 0.0,
                    duration: None,
                }),
                ..Default::default()
            },
            ..base.clone()
        });
        let to_household = run(Parameters {
            modifiers: Modifiers {
                isolation: Some(Isolation {
                    coverage: 1.0,
                    restrict_to: "household".into(),
                    delay: 0.0,
                    duration: None,
                }),
                ..Default::default()
            },
            ..base
        });
        // The run completed (bound stayed valid), and confining to the large
        // workplace transmits far more than confining to the tiny household.
        assert!(
            to_work.cum_incidence() > to_household.cum_incidence(),
            "work-confined incidence ({}) should exceed household-confined ({})",
            to_work.cum_incidence(),
            to_household.cum_incidence()
        );
    }
}
