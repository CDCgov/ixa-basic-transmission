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

/// Wall-clock time at which the person became infectious. `f64::NAN` for
/// anyone who hasn't been infected yet. ixa `Property` requires `Eq +
/// Hash`, which f64 doesn't have by default — we provide them via the
/// bit pattern. We never read this for `Susceptible` people, so the NaN
/// default is just a never-observed sentinel.
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
