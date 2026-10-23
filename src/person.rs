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

/// When the person was infected; `None` until they are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InfectionTime(pub Option<f64>);

impl_property!(InfectionTime, Person, default_const = InfectionTime(None));
