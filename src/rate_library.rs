use std::cell::OnceCell;

use ixa::data_structures::entity_map::EntityMap;
use ixa::{define_entity, impl_property};

use crate::person::Person;
use crate::rate::Curve;

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
/// `Curve` (points + precomputed cumulative). Lives in a data plugin
/// so hot-path lookups in `schedule_recovery` /
/// `schedule_next_infection_attempt` are O(1) indexed-vector reads,
/// the curve can be borrowed without cloning, and the cumulative
/// integral is built once at setup. Also stores `Vec<RateId>` so we
/// can sample a uniform-random Rate in O(1) without re-iterating the
/// map. The `empirical` slot reuses this same storage for the single-
/// curve `InfectionRate::Empirical` variant so its cum is precomputed
/// once too.
pub struct RateLibraryData {
    pub ids: Vec<RateId>,
    pub curves: EntityMap<Rate, Curve>,
    // Lazy slot for the single-curve `InfectionRate::Empirical` variant.
    // Populated on first access via `empirical_or_build` so tests that
    // bypass `setup()` still get a precomputed cum without per-call
    // allocation. `OnceCell` gives interior mutability behind `&self`,
    // which the hot path holds via `Context::get_data`.
    empirical: OnceCell<Curve>,
}

impl Default for RateLibraryData {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLibraryData {
    pub fn new() -> Self {
        Self {
            ids: Vec::new(),
            curves: EntityMap::new(),
            empirical: OnceCell::new(),
        }
    }

    /// Borrow the curve for a given assigned rate. Panics if the
    /// assignment is `None` or unknown — both are bugs (we only reach
    /// the hot path in Library mode, where every infectious person has
    /// a real assignment).
    #[inline]
    pub fn curve(&self, id: AssignedRate) -> &Curve {
        let entity = id.0.expect("AssignedRate is None in Library mode");
        self.curves
            .get(entity)
            .expect("RateId not found in library")
    }

    /// Borrow the precomputed `Curve` for `InfectionRate::Empirical`,
    /// building it from `points` on first call.
    #[inline]
    pub fn empirical_or_build(&self, points: &[[f64; 2]]) -> &Curve {
        self.empirical.get_or_init(|| Curve::new(points.to_vec()))
    }
}
