use ixa::prelude::*;
use ixa::{
    define_data_plugin, define_entity, define_global_property, define_property, define_rng, Context,
};
use rand_distr::Exp;

use crate::parameters::Parameters;
use crate::stats::ModelStats;

define_global_property!(Params, Parameters);

define_rng!(InfectionRng);
define_rng!(RecoveryRng);

define_data_plugin!(ModelStatsPlugin, ModelStats, |context| {
    let params = context.get_global_property_value(Params).unwrap();
    ModelStats::new(params.initial_infections)
});

define_entity!(Person);
define_property!(
    enum InfectionStatus {
        Susceptible,
        Infectious,
        Recovered,
    },
    Person,
    default_const = InfectionStatus::Susceptible
);

trait InfectionLoop {
    fn get_params(&self) -> &Parameters;
    fn get_stats(&self) -> &ModelStats;
    #[cfg_attr(not(test), allow(dead_code))]
    fn infected_people(&self) -> usize;
    fn random_person(&mut self) -> Option<PersonId>;
    fn infect_person(&mut self, p: PersonId, t: Option<f64>);
    fn recover_person(&mut self, p: PersonId);
    fn schedule_recovery(&mut self, p: PersonId);
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
    fn random_person(&mut self) -> Option<PersonId> {
        self.sample_entity(InfectionRng, Person)
    }
    fn infect_person(&mut self, p: PersonId, t: Option<f64>) {
        if self.get_property::<_, InfectionStatus>(p) != InfectionStatus::Susceptible {
            return;
        }
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
    fn schedule_recovery(&mut self, p: PersonId) {
        // Each infected person draws their own recovery time from
        // Exp(1 / infectious_period).
        let infectious_period = self.get_params().infectious_period;
        let dt = self.sample_distr(RecoveryRng, Exp::new(1.0 / infectious_period).unwrap());
        let recovery_time = self.get_current_time() + dt;
        self.add_plan(recovery_time, move |context| {
            if context.get_property::<_, InfectionStatus>(p) == InfectionStatus::Infectious {
                context.recover_person(p);
            }
        });
    }
    fn schedule_next_infection_attempt(&mut self, infector: PersonId) {
        // Each infectious person forecasts their own next transmission attempt
        // from Exp(infection_rate)
        let infection_rate = self.get_params().infection_rate;
        if infection_rate == 0.0 {
            return;
        }
        let dt = self.sample_distr(InfectionRng, Exp::new(infection_rate).unwrap());
        let next_time = self.get_current_time() + dt;
        self.add_plan(next_time, move |context| {
            if context.get_property::<_, InfectionStatus>(infector) != InfectionStatus::Infectious {
                return;
            }
            if let Some(target) = context.random_person() {
                let now = context.get_current_time();
                context.infect_person(target, Some(now));
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
                context.schedule_recovery(event.entity_id);
                context.schedule_next_infection_attempt(event.entity_id);
            },
        );

        for _ in 0..population {
            self.add_entity(Person).unwrap();
        }

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
    let mut ctx = Context::new();
    ctx.set_global_property_value(Params, params).unwrap();
    ctx.setup();
    ctx.execute();
    ctx.get_stats().clone()
}

#[cfg(test)]
mod tests {
    use super::*;
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
        // on each infection so the pool stays constant. Each attempt picks
        // uniformly, so the contact is hit with prob 1/2. Expected:
        //   mean count in [0, T] = rate · 0.5 · T
        //   pooled infection times ~ Uniform(0, T)
        let num_sims: u64 = 20_000;
        let infection_rate = 5.0;
        let max_time = 1.0;
        // Suppress recovery of the index case during the window.
        let infectious_period = 1e6;

        let times: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
        for seed in 0..num_sims {
            let times_clone = Rc::clone(&times);
            let params = Parameters {
                infection_rate,
                infectious_period,
                population: 2,
                initial_infections: 1,
                seed,
                max_time,
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
        let expected_mean = infection_rate * 0.5 * max_time;
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
    fn recovery_times_match_exponential() {
        // All people infected at t=0 with transmission off. Recovery delays
        // should follow Exp(1/infectious_period).
        let population: usize = 5_000;
        let infectious_period = 2.0;
        let max_time = 100.0;

        let recovery_times: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
        let recovery_times_clone = Rc::clone(&recovery_times);

        let params = Parameters {
            infection_rate: 0.0,
            infectious_period,
            population,
            initial_infections: population,
            seed: 0,
            max_time,
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
            infection_rate: 0.0,
            infectious_period: 3.0,
            population,
            initial_infections,
            seed: 0,
            max_time: 50.0,
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
        // λ = rate / pop over T, so P(contact infected) = 1 − exp(−λT) and
        // E[cum_incidence] = N · (1 − exp(−λT)).
        let num_sims: u64 = 1_000;
        let population: usize = 6;
        let n_contacts: usize = population - 1;
        let infection_rate: f64 = 3.6;
        let max_time: f64 = 5.0;
        let infectious_period: f64 = 1e6; // suppress recovery in the window

        let lambda = infection_rate / population as f64;
        let p_infected = 1.0 - f64::exp(-lambda * max_time);
        let expected_cases = n_contacts as f64 * p_infected;

        let mut total: f64 = 0.0;
        for seed in 0..num_sims {
            let params = Parameters {
                infection_rate,
                infectious_period,
                population,
                initial_infections: 1,
                seed,
                max_time,
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
                infection_rate: 0.0,
                infectious_period: 1e6,
                population,
                initial_infections,
                seed,
                max_time: 0.0,
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
}
