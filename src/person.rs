use ixa::{define_entity, define_property, impl_property};

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InfectionTime(pub f64);

impl_property!(
    InfectionTime,
    Person,
    default_const = InfectionTime(f64::NAN)
);
