use anyhow::Result;
use ixa::prelude::*;
use serde::{Deserialize, Serialize};

// Define the Person entity - this creates PersonId = EntityId<Person>
define_entity!(Person);

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone, Copy)]
pub struct InfectionData {
    pub infection_time: Option<f64>,
    pub infected_by: Option<PersonId>,
    pub recovery_time: Option<f64>,
}

// In ixa 2.0, the value type IS the property type (single type, not two)
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone, Copy)]
pub enum InfectionStatus {
    Susceptible,
    #[allow(private_interfaces)]
    Infectious(InfectionData),
    #[allow(private_interfaces)]
    Recovered(InfectionData),
}

// Declare InfectionStatus as a Property<Person> with a default value
impl_property!(
    InfectionStatus,
    Person,
    default_const = InfectionStatus::Susceptible
);

#[allow(dead_code)]
impl InfectionStatus {
    pub fn is_susceptible(&self) -> bool {
        self == &InfectionStatus::Susceptible
    }
    pub fn is_incidence(&self) -> bool {
        self.is_infectious() && self.infection_time().is_some()
    }
    pub fn infection_time(&self) -> Option<f64> {
        match self {
            InfectionStatus::Infectious(InfectionData { infection_time, .. }) => *infection_time,
            InfectionStatus::Recovered(InfectionData { infection_time, .. }) => *infection_time,
            InfectionStatus::Susceptible => None,
        }
    }
    pub fn infected_by(&self) -> Option<PersonId> {
        match self {
            InfectionStatus::Infectious(InfectionData { infected_by, .. }) => *infected_by,
            InfectionStatus::Recovered(InfectionData { infected_by, .. }) => *infected_by,
            InfectionStatus::Susceptible => None,
        }
    }
    pub fn is_infectious(&self) -> bool {
        matches!(self, InfectionStatus::Infectious { .. })
    }
    pub fn is_recovered(&self) -> bool {
        matches!(self, InfectionStatus::Recovered { .. })
    }
    pub fn to_recovered(self, recovery_time: f64) -> Result<Self> {
        match self {
            InfectionStatus::Infectious(InfectionData {
                infection_time,
                infected_by,
                ..
            }) => Ok(InfectionStatus::Recovered(InfectionData {
                infection_time,
                infected_by,
                recovery_time: Some(recovery_time),
            })),
            InfectionStatus::Recovered { .. } => anyhow::bail!("Person is already recovered"),
            InfectionStatus::Susceptible => anyhow::bail!("Person is not infectious"),
        }
    }
}
