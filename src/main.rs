mod infection_manager;
mod infection_status;
pub mod ixa_plus;
mod model;
mod output_manager;
mod params;
mod population_manager;
mod simulation_event;
mod total_infectiousness_multiplier;
mod transmission_manager;

// Helper for importing all extensions
// use crate::ext::*;
pub mod ext {
    pub use crate::infection_manager::InfectionManagerExt;
    pub use crate::output_manager::OutputManagerExt;
    pub use crate::params::ParametersExt;
    pub use crate::population_manager::PopulationManagerExt;
    pub use crate::transmission_manager::TransmissionManagerExt;
}

use crate::ixa_plus::params_macro::IxaParameters;
use crate::output_manager::OutputManagerExt;
use ixa::prelude::*;

fn main() {
    // Initialize logger
    #[cfg(debug_assertions)]
    crate::ixa_plus::log::set_log_level(crate::ixa_plus::log::LevelFilter::Debug);

    #[cfg(not(debug_assertions))]
    crate::ixa_plus::log::init_default();

    // Use mise run --params <file> to override default parameters
    let params = params::Params::from_args();
    let mut context = model::setup(params).unwrap();
    context.execute();
    context.log_stats();
}
