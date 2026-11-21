//! Settings-based contact structure: groups of people that share a
//! density-scaled contact rate.
//!
//! Each `SettingType` declares a name, an `alpha` (density exponent),
//! a discrete size distribution, and a `proportion` (itinerary weight).
//! Per-setting infectiousness multiplier is `(n − 1)^alpha` where `n`
//! is the realized member count.
//!
//! This module only builds the population: [`setup`] adds each configured
//! type as a [`Setting`] kind of ixa's `core_epi` settings, places every
//! person in exactly one group of every type (the last group of a type may
//! be undersized) and assigns everyone the itinerary given by the
//! proportions. Membership, multipliers, the itinerary and contact sampling
//! are then core_epi's; see `model::evaluate_forecast` for how the loop uses
//! them.
//!
//! Proportions are shares of where a person's attempts happen, so only
//! their ratios matter: the total multiplier is `Σ p_s · M_s / Σ p_s`.
//!
//! When `types` is empty the model falls back to global random mixing.

use ixa::core_epi::{Itinerary, SettingsExt};
use ixa::prelude::*;
use ixa::{define_rng, define_settings, Context};
use serde::{Deserialize, Serialize};

use crate::person::{Person, PersonId};

define_rng!(SettingsRng);

// One kind per `SettingType` in the parameters, added by `setup`.
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
}

/// Build the population's settings membership. Call exactly once from
/// `model::setup` after all `Person` entities exist.
pub fn setup(context: &mut Context, types: &[SettingType]) {
    let people: Vec<PersonId> = context.query_result_iterator(Person).collect();
    let pop_size = people.len();
    if types.is_empty() || pop_size == 0 {
        return;
    }

    let mut itinerary = Vec::with_capacity(types.len());
    for st in types {
        let kind = context.add_setting_kind::<Setting>(&st.name, st.alpha);
        itinerary.push((kind, st.proportion));

        // Random shuffle of the population for this type. Fisher–Yates
        // via repeated `sample_range`; each call consumes one RNG draw.
        let mut shuffled = people.clone();
        for i in (1..pop_size).rev() {
            let j: usize = context.sample_range(SettingsRng, 0..=i);
            shuffled.swap(i, j);
        }

        // Size weights stay constant across draws for this type.
        let size_weights: Vec<f64> = st.sizes.iter().map(|&(_, p)| p).collect();
        let size_values: Vec<usize> = st.sizes.iter().map(|&(s, _)| s).collect();

        let mut cursor = 0;
        while cursor < pop_size {
            let size_idx = context.sample_weighted(SettingsRng, &size_weights);
            // Last setting may be truncated to whatever's left.
            let size = size_values[size_idx].min(pop_size - cursor);
            if size == pop_size {
                // One setting holding everyone needs no member list.
                context.create_population_setting_group(kind);
            } else {
                let group = context.create_setting_group(kind);
                for &person in &shuffled[cursor..cursor + size] {
                    context.add_setting_member(group, person);
                }
            }
            cursor += size;
        }
    }
    context.set_itinerary_assignment(Itinerary::new(itinerary));
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

    fn build_context_with(types: Vec<SettingType>, population: usize, seed: u64) -> Context {
        let mut ctx = Context::new();
        ctx.init_random(seed);
        for _ in 0..population {
            ctx.add_entity(Person).unwrap();
        }
        setup(&mut ctx, &types);
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
    fn no_types_adds_no_kinds() {
        let ctx = build_context_with(Vec::new(), 5, 0);
        assert!(ctx.setting_kinds::<Setting>().is_empty());
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
            assert_almost_eq!(ctx.current_setting_multiplier::<Setting>(p), 3.0, 1e-12);
        }
    }

    #[test]
    fn proportions_are_shares_of_the_total_multiplier() {
        // home M=2, work M=4. Only the ratio of the proportions matters:
        // 0.25/0.25 and 0.9/0.9 both give (2 + 4) / 2 = 3.
        for proportion in [0.25, 0.9] {
            let ctx = build_context_with(home_and_work(proportion, proportion), 15, 0);
            let p = ctx.query_result_iterator(Person).next().unwrap();
            assert_almost_eq!(ctx.current_setting_multiplier::<Setting>(p), 3.0, 1e-9);
        }
        // 0.75/0.25: 0.75·2 + 0.25·4 = 2.5.
        let ctx = build_context_with(home_and_work(0.75, 0.25), 15, 0);
        let p = ctx.query_result_iterator(Person).next().unwrap();
        assert_almost_eq!(ctx.current_setting_multiplier::<Setting>(p), 2.5, 1e-9);
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
}
