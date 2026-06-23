use std::cell::OnceCell;

use enum_dispatch::enum_dispatch;
use ixa::impl_property;

use super::{ConstantRate, EmpiricalRate, InfectiousnessRateFn};
use crate::person::Person;

/// A resolved concrete rate function. There are only two concrete shapes in
/// this model, so we dispatch through an enum (static dispatch, inlinable)
/// rather than `Box<dyn InfectiousnessRateFn>` — the hot path calls
/// `rate`/`cum_rate`/`inverse_cum_rate` per event and a vtable indirection is
/// measurable on the no-settings configs. `#[enum_dispatch]` generates the
/// `InfectiousnessRateFn` impl (forwarding to the active variant) so there's no
/// hand-written `match` boilerplate.
#[enum_dispatch(InfectiousnessRateFn)]
#[derive(Clone, Debug)]
pub enum RateFn {
    Constant(ConstantRate),
    Empirical(EmpiricalRate),
}

/// Per-person assignment for the `Library` variant: the index into the
/// storage's `library` vector of rate functions. `None` outside Library mode
/// (the default), and never read in that case.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssignedRate(pub Option<usize>);

impl_property!(AssignedRate, Person, default_const = AssignedRate(None));

/// Storage for resolved per-person infectiousness rate functions.
///
/// - `library` / `recovery_times`: one rate function (and its recovery time)
///   per curve in a `Library` rate, built eagerly in `model::setup` (each
///   person is assigned an index into these). Empty for non-Library rates.
/// - `single`: a lazily-built slot for the single-rate `Constant` / `Empirical`
///   variants, populated on first hot-path access — so tests that build a
///   `Context` without `setup()` still resolve a rate function. Mirrors the old
///   `RateLibraryData::empirical_or_build` `OnceCell`.
pub struct RateStorage {
    library: Vec<RateFn>,
    recovery_times: Vec<f64>,
    single: OnceCell<(RateFn, f64)>,
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
            recovery_times: Vec::new(),
            single: OnceCell::new(),
        }
    }

    /// Install the per-curve library (`Library` variant only); called once at setup.
    pub fn set_library(&mut self, rate_fns: Vec<RateFn>, recovery_times: Vec<f64>) {
        self.library = rate_fns;
        self.recovery_times = recovery_times;
    }

    #[inline]
    #[must_use]
    pub fn library_len(&self) -> usize {
        self.library.len()
    }

    /// Borrow the rate function for a Library index.
    #[inline]
    #[must_use]
    pub fn library_fn(&self, idx: usize) -> &RateFn {
        &self.library[idx]
    }

    /// Recovery time for a Library index (= the curve's max-cum reach time).
    #[inline]
    #[must_use]
    pub fn recovery_time(&self, idx: usize) -> f64 {
        self.recovery_times[idx]
    }

    /// Borrow the single-rate function, building it (and its recovery time) on
    /// first access via `build`.
    #[inline]
    pub fn single_or_build(&self, build: impl FnOnce() -> (RateFn, f64)) -> &(RateFn, f64) {
        self.single.get_or_init(build)
    }
}

/// Build one `EmpiricalRate` per curve in a `Library` rate, plus the matching
/// recovery times. All curves share `scale` (per-curve heterogeneity is
/// preserved by the area normalization, not the scale).
#[must_use]
pub fn build_empirical_library(rates: &[Vec<[f64; 2]>], scale: f64) -> (Vec<RateFn>, Vec<f64>) {
    let mut rate_fns: Vec<RateFn> = Vec::with_capacity(rates.len());
    let mut recovery_times = Vec::with_capacity(rates.len());
    for curve in rates {
        let er = EmpiricalRate::new(curve.clone(), scale);
        recovery_times.push(er.recovery_time());
        rate_fns.push(RateFn::Empirical(er));
    }
    (rate_fns, recovery_times)
}
