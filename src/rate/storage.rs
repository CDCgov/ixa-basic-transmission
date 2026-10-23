use std::cell::OnceCell;

use enum_dispatch::enum_dispatch;
use ixa::data_structures::entity_map::EntityMap;
use ixa::impl_property;

use super::{ConstantRateFn, EmpiricalRateFn, InvertibleRate};
use crate::person::{Person, PersonId};

/// A resolved concrete rate function. There are only two concrete shapes in
/// this model, so we dispatch through an enum (static dispatch, inlinable)
/// rather than `Box<dyn InvertibleRate>` — the hot path calls
/// `rate`/`cum_rate`/`inverse_cum_rate` per event and a vtable indirection is
/// measurable on the no-settings configs. `#[enum_dispatch]` generates the
/// `InvertibleRate` impl (forwarding to the active variant) so there's no
/// hand-written `match` boilerplate.
///
/// Note that all variants of an enum are of the same type, so if we want to
/// take advantage of *any* variant being invertible, we must make an enum
/// in which *all* variants are invertible.
#[enum_dispatch(InvertibleRate)]
#[derive(Clone, Debug)]
pub enum InvertibleRateFn {
    Constant(ConstantRateFn),
    Empirical(EmpiricalRateFn),
}

/// Per-person assignment for the `Library` variant: the index into the
/// storage's `library` vector of rate functions, drawn when the person is
/// infected. `None` outside Library mode or before infection (the default), and
/// never read in that case.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssignedRate(pub Option<usize>);

impl_property!(AssignedRate, Person, default_const = AssignedRate(None));

/// Storage for resolved per-person infectiousness rate functions.
///
/// - `library`: one rate function per curve in a `Library` rate, built on the
///   first infection (each infected person is assigned an index into it).
///   Empty for non-Library rates.
/// - `single`: a lazily-built slot for the single-rate `Empirical` variant,
///   populated on first hot-path access — so tests that build a `Context`
///   without `setup()` still resolve a rate function. Mirrors the old
///   `RateLibraryData::empirical_or_build` `OnceCell`.
/// - `assigned`: one rate function assigned to each person when infected; used
///   by `Constant`, whose infectious period is drawn per person.
pub struct RateStorage {
    library: Vec<InvertibleRateFn>,
    single: OnceCell<InvertibleRateFn>,
    assigned: EntityMap<Person, InvertibleRateFn>,
}

impl Default for RateStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl RateStorage {
    #[must_use]
    pub fn new() -> Self {
        Self {
            library: Vec::new(),
            single: OnceCell::new(),
            assigned: EntityMap::new(),
        }
    }

    /// Store `person`'s own rate function.
    pub fn set_assigned_fn(&mut self, person: PersonId, rate_fn: InvertibleRateFn) {
        self.assigned.insert(person, rate_fn);
    }

    /// Borrow `person`'s assigned rate function, if any.
    #[inline]
    #[must_use]
    pub fn assigned_fn(&self, person: PersonId) -> Option<&InvertibleRateFn> {
        self.assigned.get(person)
    }

    /// Install the per-curve library (`Library` variant only); called once.
    pub fn set_library(&mut self, rate_fns: Vec<InvertibleRateFn>) {
        self.library = rate_fns;
    }

    #[inline]
    #[must_use]
    pub fn library_len(&self) -> usize {
        self.library.len()
    }

    /// Borrow the rate function for a Library index.
    #[inline]
    #[must_use]
    pub fn library_fn(&self, idx: usize) -> &InvertibleRateFn {
        &self.library[idx]
    }

    /// Borrow the single-rate function, building it on first access via `build`.
    #[inline]
    pub fn single_or_build(&self, build: impl FnOnce() -> InvertibleRateFn) -> &InvertibleRateFn {
        self.single.get_or_init(build)
    }
}

/// Build one `EmpiricalRateFn` per curve in a `Library` rate. All curves share
/// `scale` (per-curve heterogeneity is preserved by the area normalization, not
/// the scale).
#[must_use]
pub fn build_empirical_library(rates: &[Vec<[f64; 2]>], scale: f64) -> Vec<InvertibleRateFn> {
    rates
        .iter()
        .map(|curve| InvertibleRateFn::Empirical(EmpiricalRateFn::new(curve.clone(), scale)))
        .collect()
}
