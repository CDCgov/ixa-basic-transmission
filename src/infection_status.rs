use anyhow::Result;
use ixa::prelude::*;
use serde::{Deserialize, Serialize};

define_person_property_with_default!(InfectionStatus, Status, Status::Susceptible);
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone, Copy)]
pub struct InfectionData {
    pub infection_time: Option<f64>,
    pub infected_by: Option<PersonId>,
    pub recovery_time: Option<f64>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone, Copy)]
pub enum Status {
    Susceptible,
    #[allow(private_interfaces)]
    Infectious(InfectionData),
    #[allow(private_interfaces)]
    Recovered(InfectionData),
}

#[allow(dead_code)]
impl Status {
    pub fn is_susceptible(&self) -> bool {
        self == &Status::Susceptible
    }
    pub fn is_incidence(&self) -> bool {
        self.is_infectious() && self.infection_time().is_some()
    }
    pub fn infection_time(&self) -> Option<f64> {
        match self {
            Status::Infectious(InfectionData { infection_time, .. }) => *infection_time,
            Status::Recovered(InfectionData { infection_time, .. }) => *infection_time,
            Status::Susceptible => None,
        }
    }
    pub fn infected_by(&self) -> Option<PersonId> {
        match self {
            Status::Infectious(InfectionData { infected_by, .. }) => *infected_by,
            Status::Recovered(InfectionData { infected_by, .. }) => *infected_by,
            Status::Susceptible => None,
        }
    }
    pub fn is_infectious(&self) -> bool {
        matches!(self, Status::Infectious { .. })
    }
    pub fn is_recovered(&self) -> bool {
        matches!(self, Status::Recovered { .. })
    }
    pub fn to_recovered(self, recovery_time: f64) -> Result<Self> {
        match self {
            Status::Infectious(InfectionData {
                infection_time,
                infected_by,
                ..
            }) => Ok(Status::Recovered(InfectionData {
                infection_time,
                infected_by,
                recovery_time: Some(recovery_time),
            })),
            Status::Recovered { .. } => anyhow::bail!("Person is already recovered"),
            Status::Susceptible => anyhow::bail!("Person is not infectious"),
        }
    }
}
