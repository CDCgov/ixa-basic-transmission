use cfasim_model::{ModelOutput, model_outputs};
use ixa_basic_transmission::ext::OutputManagerExt;
use ixa_basic_transmission::model;
use ixa_basic_transmission::params::Params;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn simulate(
    population_size: usize,
    initial_infections: usize,
    max_time: f64,
    seed: u64,
) -> JsValue {
    let params: Params = Params::builder()
        .population_size(population_size)
        .initial_infections(initial_infections)
        .seed(seed)
        .max_time(max_time)
        .try_into()
        .expect("Invalid parameters");

    let mut context = model::setup(Some(params)).expect("Failed to set up model");
    context.execute();

    let daily_incidence = context.get_daily_incidence();
    let total_infections = context.get_total_infections();
    let forecasts_rejected = context.get_forecasts_rejected();

    let len = daily_incidence.len();
    let time: Vec<f64> = (0..len).map(|i| i as f64).collect();
    let incidence: Vec<f64> = daily_incidence.iter().map(|&x| x as f64).collect();

    let mut cumulative = Vec::with_capacity(len);
    let mut sum = 0.0;
    for &val in &incidence {
        sum += val;
        cumulative.push(sum);
    }

    let series = ModelOutput::new(len)
        .add_f64("time", time)
        .add_f64("incidence", incidence)
        .add_f64("cumulative_incidence", cumulative);

    let total_attempts = (total_infections + forecasts_rejected) as f64;
    let forecast_efficiency = if total_attempts > 0.0 {
        1.0 - (forecasts_rejected as f64 / total_attempts)
    } else {
        0.0
    };
    let attack_rate = total_infections as f64 / population_size as f64;

    let stats = ModelOutput::new(1)
        .add_f64("total_infections", vec![total_infections as f64])
        .add_f64("forecasts_rejected", vec![forecasts_rejected as f64])
        .add_f64("forecast_efficiency", vec![forecast_efficiency])
        .add_f64("attack_rate", vec![attack_rate]);

    model_outputs([("daily_incidence", series), ("stats", stats)])
}
