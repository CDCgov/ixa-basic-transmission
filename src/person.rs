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

/// Wall-clock time at which the person became infectious. ixa `Property`
/// requires `Eq + Hash`, which f64 lacks by default — we provide them
/// via `to_bits()` so NaN compares as equal to itself. Read only when
/// the property has been set (the NaN default is a never-observed
/// placeholder).
#[derive(Debug, Clone, Copy)]
pub struct InfectionTime(pub f64);

impl PartialEq for InfectionTime {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}
impl Eq for InfectionTime {}
impl std::hash::Hash for InfectionTime {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl_property!(
    InfectionTime,
    Person,
    default_const = InfectionTime(f64::NAN)
);
