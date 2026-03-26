// Re-export for macro use
pub use ixa::IxaError;

mod infection_manager;
mod infection_status;
pub mod ixa_plus;
pub mod model;
mod output_manager;
pub mod params;
mod population_manager;
mod simulation_event;
mod total_infectiousness_multiplier;
mod transmission_manager;

pub mod ext {
    pub use crate::infection_manager::InfectionManagerExt;
    pub use crate::output_manager::OutputManagerExt;
    pub use crate::params::ParametersExt;
    pub use crate::population_manager::PopulationManagerExt;
    pub use crate::transmission_manager::TransmissionManagerExt;
}