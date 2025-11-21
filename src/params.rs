use std::sync::LazyLock;

use crate::ixa_plus::{define_parameters, distr::gamma::*};
use anyhow::bail;
use ixa::prelude::*;

define_parameters! {
    defaults: "../params/default.toml",
    pub struct Params {
        // The number of people in the population
        population_size: usize {
            validate(value) {
                if *value == 0 {
                    bail!("population_size must be greater than 0");
                }
            }
        },

        /// Number of initial infections as a proportion of the population
        /// E.g., 0.1 means 10% of the population are initially infected
        p_initial_incidence: f64 {
            validate(value) {
                if *value < 0.0 || *value > 1.0 {
                    bail!("initial_incidence must be between 0 and 1");
                }
            }
        },

        /// The proportion of people that are initially recovered (fully immune to disease)
        /// E.g., 0.1 means 10% of the population are initially recovered
        p_initial_recovered: f64 {
            validate(value) {
                if *value < 0.0 || *value > 1.0 {
                    bail!("initial_recovered must be between 0 and 1");
                }
            }
        },

        /// The maximum run time of the simulation; even if there are still infections
        /// scheduled to occur, the simulation will stop at this time.
        max_time: f64 {
            validate(value) {
                if *value < 0.0 {
                    bail!("max_time must be non-negative");
                }
            }
        },

        /// The random seed for the simulation.
        seed: u64 {
            validate(value) {
                if *value == 0 {
                    bail!("seed must be non-zero");
                }
            }
        },

        /// The distribution of infection rates across the population
        infection_rate: Gamma,

        /// The distribution of infection durations across the population
        infection_duration: Gamma,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_rate_distributions() {
        let params: Params = Params::builder()
            .infection_duration(gamma!(shape = 3.0, rate = 0.5).unwrap())
            .try_into()
            .unwrap();
        let mut rng = rand::rng();
        params.infection_duration.sample(&mut rng);
        assert_eq!(params.infection_duration.scale(), 2.0, "scale");
        assert_eq!(params.infection_duration.mean(), 1.5, "mean");
    }
}
