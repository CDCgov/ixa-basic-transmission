use ixa::data_structures::entity_map::EntityMap;
use ixa::{define_entity, impl_property};

use crate::person::Person;

// A `Rate` is an entity that owns one empirical infectiousness curve.
// Each person in Library mode is randomly assigned exactly one Rate at
// setup and uses its curve for the duration of their infectious period.
// `define_entity!` also generates `pub type RateId = EntityId<Rate>` via
// `paste!`, which we use as the per-person assignment.
define_entity!(Rate);

/// Per-person assignment: which `Rate`'s curve this person draws their
/// infectiousness profile from. `None` outside Library mode (the
/// default), and never read in that case.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssignedRate(pub Option<RateId>);

impl_property!(AssignedRate, Person, default_const = AssignedRate(None));

/// Storage for the rate library: an `EntityMap` keyed by `RateId` →
/// curve. Lives in a data plugin so hot-path lookups in
/// `schedule_recovery` / `schedule_next_infection_attempt` are O(1)
/// indexed-vector reads, and the curve slice can be borrowed without
/// cloning. Also stores `Vec<RateId>` so we can sample a uniform-random
/// Rate in O(1) without re-iterating the map.
pub struct RateLibraryData {
    pub ids: Vec<RateId>,
    pub curves: EntityMap<Rate, Vec<[f64; 2]>>,
}

impl RateLibraryData {
    pub fn new() -> Self {
        Self {
            ids: Vec::new(),
            curves: EntityMap::new(),
        }
    }

    /// Borrow the curve for a given assigned rate. Panics if the
    /// assignment is `None` or unknown — both are bugs (we only reach
    /// the hot path in Library mode, where every infectious person has
    /// a real assignment).
    #[inline]
    pub fn curve(&self, id: AssignedRate) -> &[[f64; 2]] {
        let entity = id.0.expect("AssignedRate is None in Library mode");
        self.curves
            .get(entity)
            .expect("RateId not found in library")
            .as_slice()
    }
}
