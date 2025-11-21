use crate::ext::*;
use crate::infection_status::*;
use crate::ixa_plus::rate_fn::*;
use crate::simulation_event::SimulationEvent;
use crate::total_infectiousness_multiplier;
use anyhow::Result;
use ixa::prelude::*;
use rand_distr::Exp;

define_rng!(InfectionRng);
define_rng!(ForecastRng);

define_rate!(InfectionRate, |context, _person_id| {
    let r_distr = context.param_infection_rate();
    let duration_distr = context.param_infection_duration();
    let params = ConstantRateParams {
        r: context.sample_distr(InfectionRng, r_distr),
        infection_duration: context.sample_distr(InfectionRng, duration_distr),
    };
    log::trace!("Assigning infection rate: {params:?}");
    RateFn::ConstantRate(params.try_into().unwrap())
});

pub trait InfectionManagerExt: PluginContext {
    /// Schedule a forecast for the next infection time for a person.
    fn schedule_infection_loop(&mut self, person_id: PersonId) -> Result<()> {
        // Get the time elapsed since the person became infectious
        // Returns an error if the person is not infectious
        let elapsed = self.get_elapsed_infection_time(person_id)?;

        // Get the person's individual infection rate function representing their
        // intrinsic infectiousness, which is calculated by the InfectionRate generator
        let rate_fn = self.get_person_rate_fn(person_id, InfectionRate);

        // Scale infectiousness by the maximum possible infectiousness multiplier
        let scale = total_infectiousness_multiplier::forecasted_maximum(self, person_id);

        // Apply the scale and elapsed time to the rate function
        let total_rate_fn = ScaledRateFn::new(rate_fn, scale, elapsed);

        // Draw an exponential and use that to determine the next time
        let sample = self.sample_distr(InfectionRng, Exp::new(1.0).unwrap());

        let next_time_diff = total_rate_fn.inverse_cum_rate(sample);

        let Some(next_time_diff) = next_time_diff else {
            // If the function is not able to return a time, it means that the person
            // is no longer infectious, so we exit the loop
            return Ok(());
        };

        let forecasted_total_infectiousness = total_rate_fn.rate(next_time_diff);

        if !(forecasted_total_infectiousness > 0.0) {
            // The person is no longer infectious, exit the loop
            return Ok(());
        }

        let next_time = self.get_current_time() + next_time_diff;

        self.add_plan(next_time, move |context| {
            if context.evaluate_forecast(person_id, forecasted_total_infectiousness) {
                context.attempt_transmission(person_id);
            }
            // Continue scheduling forecasts until the person recovers.
            context.schedule_infection_loop(person_id).unwrap()
        });
        Ok(())
    }

    /// Evaluates a forecast against the actual current infectious,
    /// Returns a contact to be infected or None if the forecast is rejected
    fn evaluate_forecast(
        &mut self,
        person_id: PersonId,
        forecasted_total_infectiousness: f64,
    ) -> bool {
        let rate_fn = self.get_person_rate_fn(person_id, InfectionRate);

        let total_multiplier = total_infectiousness_multiplier::actual(self, person_id);
        let total_rate_fn = ScaledRateFn::new(rate_fn, total_multiplier, 0.0);

        let elapsed_t = self.get_elapsed_infection_time(person_id).unwrap();
        let current_infectiousness = total_rate_fn.rate(elapsed_t);

        assert!(
            // 1e-10 is a small enough tolerance for floating point comparison.
            current_infectiousness <= forecasted_total_infectiousness + 1e-10,
            "Person {person_id}: Forecasted infectiousness must always be greater than or equal to current infectiousness. Current: {current_infectiousness}, Forecasted: {forecasted_total_infectiousness}"
        );

        // If they are less infectious as we expected...
        if current_infectiousness < forecasted_total_infectiousness {
            // Reject with the ratio of current vs the forecasted
            if !self.sample_bool(
                ForecastRng,
                current_infectiousness / forecasted_total_infectiousness,
            ) {
                self.emit_event(SimulationEvent::ForecastRejected {
                    t: self.get_current_time(),
                    person_id,
                });
                return false;
            }
        }

        true
    }

    /// Schedule the recovery time for an infected person
    fn schedule_recovery(&mut self, person: PersonId) {
        let infection_duration = self
            .get_person_rate_fn(person, InfectionRate)
            .infection_duration();
        let recovery_time = self.get_current_time() + infection_duration;
        self.add_plan(recovery_time, move |context| {
            context.recover_person(person, Some(recovery_time)).unwrap()
        });
    }

    /// Assigns a person's status to infected and starts the infection loop
    /// If the person was infected at the start of the simulation, the infection time
    /// and infected_by fields will not exist
    fn infect_person(
        &mut self,
        person_id: PersonId,
        infected_by: Option<PersonId>,
        infection_time: Option<f64>,
    ) {
        self.assign_rate(person_id, InfectionRate);
        self.set_person_property(
            person_id,
            InfectionStatus,
            Status::Infectious(InfectionData {
                infection_time: infection_time,
                infected_by,
                recovery_time: None,
            }),
        );

        // Start the loop
        self.schedule_infection_loop(person_id).unwrap();
        self.schedule_recovery(person_id);
    }

    /// Assigns a person's status to recovered. If the person was recovered, there
    /// will be no associated metadata about the infection and recovery time
    fn recover_person(&mut self, person_id: PersonId, recovery_time: Option<f64>) -> Result<()> {
        let status = self.get_person_property(person_id, InfectionStatus);

        self.set_person_property(
            person_id,
            InfectionStatus,
            match recovery_time {
                Some(recovery_time) => status.to_recovered(recovery_time)?,
                // The person was initially recovered, so we have no data about the infection
                None => Status::Recovered(InfectionData {
                    infection_time: None,
                    infected_by: None,
                    recovery_time: None,
                }),
            },
        );
        Ok(())
    }

    fn get_elapsed_infection_time(&self, person_id: PersonId) -> Result<f64> {
        let Status::Infectious(InfectionData { infection_time, .. }) =
            self.get_person_property(person_id, InfectionStatus)
        else {
            anyhow::bail!("Person {person_id} is not infectious");
        };
        Ok(self.get_current_time() - infection_time.unwrap_or(0.0))
    }
}

impl<C> InfectionManagerExt for C where C: PluginContext {}
