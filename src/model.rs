use crate::{ext::*, params::Params};
use anyhow::Result;
use ixa::prelude::*;

pub fn setup(params_override: Option<Params>) -> Result<Context> {
    let mut context = Context::new();

    let params = if let Some(override_params) = params_override {
        context.set_params(override_params)
    } else {
        context.use_default_params()
    };

    // Log parameters
    crate::ixa_plus::log::info!("\nRunning model with parameters:\n{}", params);

    let &Params {
        max_time,
        seed,
        population_size,
        p_initial_recovered,
        p_initial_incidence,
        ..
    } = params;

    // Set the random seed.
    context.init_random(seed);

    // Initialize output capture before creating the population so we don't miss creation events.
    context.capture_output();

    // Add a plan to shut down the simulation after `max_time`, regardless of
    // what else is happening in the model.
    context.add_plan(max_time, |context| {
        context.shutdown();
    });

    // Seed the population with initial infected/recovered individuals
    context.init_population(
        population_size,
        vec![
            (
                p_initial_recovered,
                ("Initial Recovered", |context, person_id| {
                    context.recover_person(person_id, None)?;
                    Ok(())
                }),
            ),
            // Infecting each person will kick off an infection loop which schedules their
            // next forecasted infection, as well as their recovery time.
            // See infection_manager.rs for details
            (
                p_initial_incidence,
                ("Initial Infected", |context, person_id| {
                    context.infect_person(person_id, None, None);
                    Ok(())
                }),
            ),
        ],
    )?;

    Ok(context)
}
