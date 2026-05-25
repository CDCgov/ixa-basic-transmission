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
    let mut ctx = Context::new();
    ctx.set_global_property_value(Params, params).unwrap();
    ctx.setup();
    ctx.execute();
    ctx.get_stats().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
