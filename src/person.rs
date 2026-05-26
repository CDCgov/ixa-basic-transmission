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

/// Declare a `Person` property that wraps a single `f64` with NaN as the
/// default sentinel. ixa `Property` requires `Eq + Hash`, which f64 lacks
/// by default — we provide them via `to_bits()` so NaN compares as equal
/// to itself. Read these only when you know the property has been set
/// (the NaN default is a never-observed placeholder).
macro_rules! define_nan_f64_property {
    ($(#[$attr:meta])* $name:ident) => {
        $(#[$attr])*
        #[derive(Debug, Clone, Copy)]
        pub struct $name(pub f64);

        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                self.0.to_bits() == other.0.to_bits()
            }
        }
        impl Eq for $name {}
        impl std::hash::Hash for $name {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                self.0.to_bits().hash(state);
            }
        }

        impl_property!($name, Person, default_const = $name(f64::NAN));
    };
}

define_nan_f64_property! {
    /// Wall-clock time at which the person became infectious.
    InfectionTime
}

define_nan_f64_property! {
    /// Per-person constant infectiousness rate sampled from
    /// `Gamma { rate, .. }` at setup. Unused outside the Gamma variant.
    DrawnRate
}

define_nan_f64_property! {
    /// Per-person deterministic infectious-period length sampled from
    /// `Gamma { duration, .. }` at setup. Unused outside the Gamma variant.
    DrawnDuration
}
