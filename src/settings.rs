//! Settings-based contact structure: groups of people that share a
//! density-scaled contact rate.
//!
//! Each `SettingType` declares a name, an `alpha` (density exponent),
//! a discrete size distribution, and a `proportion` (itinerary weight).
//! Per-setting infectiousness multiplier is `(n − 1)^alpha` where `n`
//! is the realized member count.
//!
//! At setup, every person is placed in exactly one `Setting` of every
//! configured type (the last setting of a type may be undersized).
//!
//! Membership, itineraries, multipliers and contact sampling are ixa's
//! `core_epi` settings: each configured type is added as a [`Setting`] kind
//! at setup (at most `MAX_SETTINGS`).
//!
//! Forecast / evaluation split (see `model::evaluate_forecast`):
//!
//! * The total multiplier is `Σ p_s · M_s`; the attempt's setting is drawn
//!   by `p_s · M_s`, giving attempts in setting `s` at rate `r(τ) · p_s · M_s`.
//! * `settings_multiplier(person)` is that total exactly, so no
//!   proposal is rejected. Membership is static; the one thing that changes a
//!   person's total is an itinerary restriction (isolation) starting or
//!   ending, and both re-forecast their transmission loop.
//!
//! When `types` is empty the model falls back to global random mixing.

use ixa::core_epi::{Itinerary, SettingsExt, TransmissionManager};
use ixa::data_structures::entity_map::EntityMap;
use ixa::prelude::*;
use ixa::{define_data_plugin, define_rng, define_settings, Context};
use serde::{Deserialize, Serialize};

use crate::person::{Person, PersonId};

define_rng!(SettingsRng);

// One kind per `SettingType` in the parameters, added at setup.
define_settings!(pub Setting for Person);

/// Single setting-type configuration entry. `proportion` is the
/// itinerary weight (proportions across types are renormalized at
/// sample time so they don't have to sum exactly to 1).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingType {
    pub name: String,
    #[serde(default = "default_alpha")]
    pub alpha: f64,
    pub sizes: Vec<(usize, f64)>,
    pub proportion: f64,
}

fn default_alpha() -> f64 {
    1.0
}

impl SettingType {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("setting name must not be empty".into());
        }
        if !self.alpha.is_finite() || self.alpha < 0.0 {
            return Err(format!(
                "setting {}: alpha must be finite and non-negative, got {}",
                self.name, self.alpha
            ));
        }
        if !self.proportion.is_finite() || self.proportion < 0.0 {
            return Err(format!(
                "setting {}: proportion must be finite and non-negative, got {}",
                self.name, self.proportion
            ));
        }
        if self.sizes.is_empty() {
            return Err(format!(
                "setting {}: size distribution must not be empty",
                self.name
            ));
        }
        let mut total = 0.0_f64;
        for (size, prob) in &self.sizes {
            if *size == 0 {
                return Err(format!(
                    "setting {}: size must be at least 1, got 0",
                    self.name
                ));
            }
            if !prob.is_finite() || *prob < 0.0 {
                return Err(format!(
                    "setting {}: probability must be finite and non-negative, got {prob}",
                    self.name
                ));
            }
            total += *prob;
        }
        if (total - 1.0).abs() > 1e-6 {
            return Err(format!(
                "setting {}: size probabilities sum to {total}, expected 1.0",
                self.name
            ));
        }
        Ok(())
    }

    /// `(n − 1)^alpha`. Zero for `n ≤ 1` (no contacts possible).
    #[inline]
    pub fn multiplier(&self, size: usize) -> f64 {
        if size <= 1 {
            0.0
        } else {
            (size as f64 - 1.0).powf(self.alpha)
        }
    }
}

/// What the model keeps on top of core_epi's settings store.
#[derive(Default)]
pub struct SettingsData {
    /// The kind of each configured type, in order; empty means settings
    /// are disabled.
    kinds: Vec<Setting>,
    /// `Σ p_s`. core_epi normalizes an itinerary's proportions, and this
    /// model's rate is `Σ p_s · M_s` with the proportions as given.
    proportion_total: f64,
    /// People whose itinerary is restricted to one setting (isolation).
    restriction: EntityMap<Person, Setting>,
}

define_data_plugin!(SettingsPlugin, SettingsData, |_context| {
    SettingsData::default()
});

pub trait ModelSettingsExt {
    /// Build the population's settings membership. Call exactly once
    /// from `model::setup` after all `Person` entities exist.
    fn settings_setup(&mut self, types: &[SettingType]);

    /// Whether any setting types are configured.
    fn settings_enabled(&self) -> bool;

    /// Factor on core_epi's (normalized) current multiplier in `person`'s
    /// total rate: `Σ p_s`, or 1 while their itinerary is restricted.
    fn settings_rate_scale(&self, person: PersonId) -> f64;

    /// `person`'s total settings multiplier now,
    /// `settings_rate_scale · current_setting_multiplier`. It is also the
    /// forecast: it only changes when a restriction starts or ends, and both
    /// re-forecast. Returns 1.0 when settings are disabled.
    fn settings_multiplier(&self, person: PersonId) -> f64;

    /// Restrict `person`'s current itinerary to their setting of type
    /// `type_index` (isolation): attempts happen only there, at its full
    /// multiplier. They stay a member of their other settings, so others can
    /// still contact them. No-op when settings are disabled or `person` has
    /// no setting of that type.
    fn settings_restrict_to(&mut self, person: PersonId, type_index: usize);

    /// Remove any itinerary restriction on `person`, restoring the full
    /// itinerary. No-op if `person` wasn't restricted.
    fn settings_clear_restriction(&mut self, person: PersonId);

    /// Whether `person` currently has an itinerary restriction.
    fn settings_is_restricted(&self, person: PersonId) -> bool;
}

impl ModelSettingsExt for Context {
    fn settings_setup(&mut self, types: &[SettingType]) {
        if types.is_empty() {
            return;
        }
        let people: Vec<PersonId> = self.query_result_iterator(Person).collect();
        let pop_size = people.len();
        if pop_size == 0 {
            return;
        }
        self.get_data_mut(SettingsPlugin).proportion_total =
            types.iter().map(|t| t.proportion).sum();

        for st in types {
            let kind = self.add_setting_kind::<Setting>(&st.name, st.alpha);
            self.get_data_mut(SettingsPlugin).kinds.push(kind);

            // Random shuffle of the population for this type. Fisher–Yates
            // via repeated `sample_range`; each call consumes one RNG draw.
            let mut shuffled = people.clone();
            for i in (1..pop_size).rev() {
                let j: usize = self.sample_range(SettingsRng, 0..=i);
                shuffled.swap(i, j);
            }

            // Size weights stay constant across draws for this type.
            let size_weights: Vec<f64> = st.sizes.iter().map(|&(_, p)| p).collect();
            let size_values: Vec<usize> = st.sizes.iter().map(|&(s, _)| s).collect();

            let mut cursor = 0;
            while cursor < pop_size {
                let size_idx = self.sample_weighted(SettingsRng, &size_weights);
                // Last setting may be truncated to whatever's left.
                let size = size_values[size_idx].min(pop_size - cursor);
                if size == pop_size {
                    // One setting holding everyone needs no member list.
                    self.create_population_setting_group(kind);
                } else {
                    let group = self.create_setting_group(kind);
                    for &person in &shuffled[cursor..cursor + size] {
                        self.add_setting_member(group, person);
                    }
                }
                cursor += size;
            }
        }

        let kinds = self.get_data(SettingsPlugin).kinds.clone();
        let proportions = types.iter().map(|st| st.proportion);
        let itinerary = Itinerary::new(kinds.into_iter().zip(proportions));
        self.set_itinerary_assignment(itinerary);
    }

    fn settings_enabled(&self) -> bool {
        !self.get_data(SettingsPlugin).kinds.is_empty()
    }

    fn settings_rate_scale(&self, person: PersonId) -> f64 {
        let data = self.get_data(SettingsPlugin);
        if data.restriction.contains_key(person) {
            1.0
        } else {
            data.proportion_total
        }
    }

    fn settings_multiplier(&self, person: PersonId) -> f64 {
        let data = self.get_data(SettingsPlugin);
        if data.kinds.is_empty() {
            return 1.0;
        }
        self.settings_rate_scale(person) * self.current_setting_multiplier::<Setting>(person)
    }

    fn settings_restrict_to(&mut self, person: PersonId, type_index: usize) {
        let Some(&kind) = self.get_data(SettingsPlugin).kinds.get(type_index) else {
            return;
        };
        if self.setting_group(person, kind).is_none() {
            return;
        }
        self.set_itinerary(person, Itinerary::new([(kind, 1.0)]));
        self.get_data_mut(SettingsPlugin)
            .restriction
            .insert(person, kind);
        // Their total may now exceed the forecast their pending proposal
        // was drawn from.
        self.reforecast_transmission(person);
    }

    fn settings_clear_restriction(&mut self, person: PersonId) {
        if self
            .get_data_mut(SettingsPlugin)
            .restriction
            .remove(person)
            .is_some()
        {
            self.clear_itinerary::<Setting>(person);
            self.reforecast_transmission(person);
        }
    }

    fn settings_is_restricted(&self, person: PersonId) -> bool {
        self.get_data(SettingsPlugin)
            .restriction
            .contains_key(person)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ixa::assert_almost_eq;

    fn make_type(name: &str, alpha: f64, sizes: Vec<(usize, f64)>, proportion: f64) -> SettingType {
        SettingType {
            name: name.into(),
            alpha,
            sizes,
            proportion,
        }
    }

    #[test]
    fn validate_rejects_bad_inputs() {
        let bad_alpha = make_type("a", -0.1, vec![(2, 1.0)], 1.0);
        assert!(bad_alpha.validate().is_err());

        let bad_nan_alpha = make_type("a", f64::NAN, vec![(2, 1.0)], 1.0);
        assert!(bad_nan_alpha.validate().is_err());

        let bad_proportion = make_type("a", 1.0, vec![(2, 1.0)], -0.1);
        assert!(bad_proportion.validate().is_err());

        let empty_sizes = make_type("a", 1.0, vec![], 1.0);
        assert!(empty_sizes.validate().is_err());

        let bad_size = make_type("a", 1.0, vec![(0, 1.0)], 1.0);
        assert!(bad_size.validate().is_err());

        let bad_probs = make_type("a", 1.0, vec![(2, 0.3), (3, 0.5)], 1.0);
        assert!(bad_probs.validate().is_err());

        let empty_name = make_type("", 1.0, vec![(2, 1.0)], 1.0);
        assert!(empty_name.validate().is_err());
    }

    #[test]
    fn multiplier_formula() {
        let st = make_type("a", 1.0, vec![(2, 1.0)], 1.0);
        assert_eq!(st.multiplier(0), 0.0);
        assert_eq!(st.multiplier(1), 0.0);
        assert_eq!(st.multiplier(2), 1.0);
        assert_eq!(st.multiplier(5), 4.0);

        let st_half = make_type("b", 0.5, vec![(2, 1.0)], 1.0);
        assert_almost_eq!(st_half.multiplier(5), 2.0, 1e-12);

        let st_zero = make_type("c", 0.0, vec![(2, 1.0)], 1.0);
        assert_eq!(st_zero.multiplier(5), 1.0); // (5-1)^0 = 1
    }

    fn build_context_with(types: Vec<SettingType>, population: usize, seed: u64) -> Context {
        let mut ctx = Context::new();
        ctx.init_random(seed);
        for _ in 0..population {
            ctx.add_entity(Person).unwrap();
        }
        ctx.settings_setup(&types);
        ctx
    }

    /// The kind of the `index`th configured type.
    fn kind(ctx: &Context, index: usize) -> Setting {
        ctx.setting_kinds::<Setting>()[index]
    }

    fn home_and_work(home_proportion: f64, work_proportion: f64) -> Vec<SettingType> {
        // home: size 3, alpha 1 → M=2; work: size 5, alpha 1 → M=4.
        vec![
            make_type("home", 1.0, vec![(3, 1.0)], home_proportion),
            make_type("work", 1.0, vec![(5, 1.0)], work_proportion),
        ]
    }

    #[test]
    fn disabled_returns_neutral_values() {
        let ctx = build_context_with(Vec::new(), 5, 0);
        assert!(!ctx.settings_enabled());
        for p in ctx.query_result_iterator(Person) {
            assert_eq!(ctx.settings_multiplier(p), 1.0);
        }
    }

    #[test]
    fn setup_assigns_everyone_one_setting_per_type() {
        // Two types, deterministic size = 3.
        let t1 = make_type("home", 1.0, vec![(3, 1.0)], 0.5);
        let t2 = make_type("work", 0.5, vec![(3, 1.0)], 0.5);
        let ctx = build_context_with(vec![t1, t2], 9, 0);
        for p in ctx.query_result_iterator(Person) {
            for slot in [kind(&ctx, 0), kind(&ctx, 1)] {
                assert!(ctx.setting_group(p, slot).is_some());
                assert_eq!(ctx.setting_group_size(p, slot), 3);
            }
        }
    }

    #[test]
    fn setup_handles_undersized_last_setting() {
        // Size 4 with population 9 → settings of size [4, 4, 1].
        let t = make_type("home", 1.0, vec![(4, 1.0)], 1.0);
        let ctx = build_context_with(vec![t], 9, 0);
        let mut sizes: Vec<usize> = ctx
            .query_result_iterator(Person)
            .map(|p| ctx.setting_group_size(p, kind(&ctx, 0)))
            .collect();
        sizes.sort_unstable();
        assert_eq!(sizes, vec![1, 4, 4, 4, 4, 4, 4, 4, 4]);
    }

    #[test]
    fn a_type_holding_everyone_is_a_population_group() {
        let t = make_type("community", 0.0, vec![(100_000, 1.0)], 1.0);
        let ctx = build_context_with(vec![t], 12, 0);
        for p in ctx.query_result_iterator(Person) {
            assert_eq!(ctx.setting_group_size(p, kind(&ctx, 0)), 12);
            assert_eq!(ctx.setting_multiplier(p, kind(&ctx, 0)), 1.0);
        }
    }

    #[test]
    fn multipliers_match_expected_for_known_assignment() {
        // Single type, single size (4), alpha=1: M = (4-1)^1 = 3.
        let t = make_type("home", 1.0, vec![(4, 1.0)], 1.0);
        let ctx = build_context_with(vec![t], 8, 0);
        for p in ctx.query_result_iterator(Person) {
            assert_almost_eq!(ctx.setting_multiplier(p, kind(&ctx, 0)), 3.0, 1e-12);
            assert_almost_eq!(ctx.settings_multiplier(p), 3.0, 1e-12);
        }
    }

    #[test]
    fn forecast_multiplier_is_the_exact_total() {
        // 0.25·2 + 0.25·4 = 1.5.
        let ctx = build_context_with(home_and_work(0.25, 0.25), 15, 0);
        let p = ctx.query_result_iterator(Person).next().unwrap();
        assert_almost_eq!(ctx.settings_rate_scale(p), 0.5, 1e-12);
        assert_almost_eq!(ctx.settings_multiplier(p), 1.5, 1e-9);
    }

    #[test]
    fn sample_contact_picks_only_setting_members_excluding_self() {
        // Two settings of size 3: a contact shares the infector's setting.
        let t = make_type("home", 1.0, vec![(3, 1.0)], 1.0);
        let ctx = build_context_with(vec![t], 6, 7);
        let p0 = ctx.query_result_iterator(Person).next().unwrap();
        let home = ctx.setting_group(p0, kind(&ctx, 0));
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let pick = ctx
                .sample_setting_contact(p0, kind(&ctx, 0))
                .expect("expected a contact");
            assert_ne!(pick, p0);
            assert_eq!(ctx.setting_group(pick, kind(&ctx, 0)), home);
            seen.insert(pick);
        }
        assert_eq!(seen.len(), 2, "did not exercise all members");
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn sample_setting_distribution_matches_p_times_m() {
        // home: M=2, p=0.8 → 1.6; work: M=4, p=0.1 → 0.4. P(home) → 0.8.
        let ctx = build_context_with(home_and_work(0.8, 0.1), 15, 0);
        let p = ctx.query_result_iterator(Person).next().unwrap();
        let mut home = 0usize;
        let n = 20_000;
        for _ in 0..n {
            if ctx.sample_setting_by_rate::<Setting>(p).unwrap() == kind(&ctx, 0) {
                home += 1;
            }
        }
        let p_home = home as f64 / n as f64;
        // 20k Bernoulli(0.8) samples; ±0.02 is generous (~7σ).
        assert!(
            (p_home - 0.8).abs() < 0.02,
            "expected P(home) ≈ 0.8, observed {p_home}"
        );
    }

    #[test]
    fn restrict_to_confines_the_itinerary_and_clears() {
        let mut ctx = build_context_with(home_and_work(0.25, 0.25), 15, 7);
        let p = ctx.query_result_iterator(Person).next().unwrap();

        // Restrict to home (type index 0): attempts only there, at its full
        // multiplier of 2.
        ctx.settings_restrict_to(p, 0);
        assert!(ctx.settings_is_restricted(p));
        assert_eq!(ctx.settings_rate_scale(p), 1.0);
        assert_almost_eq!(ctx.settings_multiplier(p), 2.0, 1e-9);
        let home = ctx.setting_group(p, kind(&ctx, 0));
        for _ in 0..200 {
            assert_eq!(
                ctx.sample_setting_by_rate::<Setting>(p),
                Some(kind(&ctx, 0))
            );
            let c = ctx
                .sample_setting_contact(p, kind(&ctx, 0))
                .expect("home setting has other members");
            assert_ne!(c, p);
            assert_eq!(ctx.setting_group(c, kind(&ctx, 0)), home);
        }
        // Still a member of work, so others can contact them there.
        assert!(ctx.setting_group(p, kind(&ctx, 1)).is_some());

        // Clearing restores the full itinerary.
        ctx.settings_clear_restriction(p);
        assert!(!ctx.settings_is_restricted(p));
        assert_eq!(ctx.settings_rate_scale(p), 0.5);
        let drew_work =
            (0..200).any(|_| ctx.sample_setting_by_rate::<Setting>(p) == Some(kind(&ctx, 1)));
        assert!(drew_work, "work is back in the itinerary");
    }

    #[test]
    fn restrict_is_noop_when_disabled() {
        let mut ctx = build_context_with(Vec::new(), 5, 0);
        let p = ctx.query_result_iterator(Person).next().unwrap();
        ctx.settings_restrict_to(p, 0);
        assert!(!ctx.settings_is_restricted(p));
        assert_eq!(ctx.settings_multiplier(p), 1.0);
    }
}
