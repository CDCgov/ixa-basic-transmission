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
//! Forecast / evaluation split:
//!
//! * `settings_max_multiplier(person)` = `max_s M_s` — single peak setting.
//! * `settings_current_multiplier(person)` = `Σ p_s · M_s` — weighted total.
//! * `settings_sample_current_setting(person)` weights settings by
//!   `p_s · M_s` (rate-weighted, not by `p_s` alone).
//!
//! The thinning ratio `current/max` collapses sampling on the upper-bound
//! process back to the true marginal rate `r(τ) · Σ p_s · M_s`.
//!
//! When `types` is empty the plugin is a no-op: multipliers are 1.0 and
//! `settings_sample_contact` falls back to global random mixing.

use ixa::data_structures::entity_map::EntityMap;
use ixa::prelude::*;
use ixa::random::sample_single_excluding;
use ixa::{define_data_plugin, define_entity, define_rng, impl_property, Context};
use serde::{Deserialize, Serialize};

use crate::person::{Person, PersonId};

define_rng!(SettingsRng);
define_rng!(SettingsContactRng);

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

define_entity!(Setting);

/// Which `SettingType` (by index into `SettingsData.types`) this
/// `Setting` is an instance of.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct SettingTypeIndex(pub usize);
impl_property!(
    SettingTypeIndex,
    Setting,
    default_const = SettingTypeIndex(0)
);

/// Storage for the settings system. Empty `types` means the plugin is
/// disabled — every accessor short-circuits and the model falls back
/// to global random mixing.
///
/// Every per-setting and per-person quantity that the hot path needs
/// is precomputed once at setup: membership doesn't change, so the
/// `powf`s and the proportion-weighted sums are stable for the run.
pub struct SettingsData {
    pub types: Vec<SettingType>,
    /// Member lists per setting.
    pub members: EntityMap<Setting, Vec<PersonId>>,
    /// `(n_s − 1)^α_s`, indexed by `SettingId`. One `powf` per setting,
    /// computed at setup; the hot path reads it back as a scalar.
    pub setting_multiplier: EntityMap<Setting, f64>,
    /// Reverse lookup: a person's settings, in the same order as
    /// `types`, so `person_to_settings[person][i]` is this person's
    /// setting of `types[i]`.
    pub person_to_settings: EntityMap<Person, Vec<SettingId>>,
    /// `max_s M_s` per person — forecast upper bound.
    pub max_multiplier: EntityMap<Person, f64>,
    /// `Σ p_s · M_s` per person — thinning numerator.
    pub current_multiplier: EntityMap<Person, f64>,
    /// `[p_s · M_s]` per person, parallel to `person_to_settings`.
    /// Reused directly by `sample_weighted` to avoid allocating a
    /// fresh `Vec` every transmission attempt.
    pub selection_weights: EntityMap<Person, Vec<f64>>,
    /// Optional per-person itinerary restriction (isolation): while present,
    /// the person's *current* multiplier and contact sampling are confined
    /// to this single setting. The forecast bound (`max_multiplier`) is left
    /// untouched, so the thinning invariant still holds. Set/cleared at
    /// runtime by `settings_restrict_to` / `settings_clear_restriction`.
    pub restriction: EntityMap<Person, SettingId>,
    /// Whether any itinerary restriction can occur this run. Armed once at
    /// setup via `settings_arm_restriction` by an intervention that may call
    /// `settings_restrict_to` (isolation). When `false`, no person is ever
    /// restricted, so `current_multiplier` is constant and is an *exact*
    /// forecast bound — the model forecasts on it directly with no thinning
    /// waste. When `true`, a restriction can raise a person's rate up to
    /// `max_multiplier`, so the forecast must use that looser bound. Read via
    /// `settings_forecast_multiplier`.
    pub restriction_armed: bool,
}

impl Default for SettingsData {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsData {
    pub fn new() -> Self {
        Self {
            types: Vec::new(),
            members: EntityMap::new(),
            setting_multiplier: EntityMap::new(),
            person_to_settings: EntityMap::new(),
            max_multiplier: EntityMap::new(),
            current_multiplier: EntityMap::new(),
            selection_weights: EntityMap::new(),
            restriction: EntityMap::new(),
            restriction_armed: false,
        }
    }

    #[inline]
    pub fn enabled(&self) -> bool {
        !self.types.is_empty()
    }
}

define_data_plugin!(SettingsPlugin, SettingsData, |_context| {
    SettingsData::new()
});

pub trait SettingsExt {
    /// Build the population's settings membership. Call exactly once
    /// from `model::setup` after all `Person` entities exist — a second
    /// call would add a fresh set of `Setting` entities on top.
    fn settings_setup(&mut self, types: &[SettingType]);

    /// `max_s M_s` for `person`. Returns 1.0 when settings are disabled.
    fn settings_max_multiplier(&self, person: PersonId) -> f64;

    /// `Σ p_s · M_s` for `person`. Returns 1.0 when settings are disabled.
    fn settings_current_multiplier(&self, person: PersonId) -> f64;

    /// Arm the itinerary-restriction mechanism for this run. Call once at
    /// setup from any intervention that may call `settings_restrict_to`
    /// (isolation). Tells `settings_forecast_multiplier` it must fall back to
    /// the loose `max_s M_s` bound, since a restriction can raise a person's
    /// current multiplier above their weighted mean.
    fn settings_arm_restriction(&mut self);

    /// Per-person forecast upper bound for the transmission NHPP. When no
    /// restriction can occur (`restriction_armed == false`), returns the
    /// *exact* constant total rate `Σ p_s · M_s` — a tight bound that makes
    /// the thinning step a no-op. When restriction is armed, returns the
    /// looser `max_s M_s` (a restricted person may transiently transmit at a
    /// single setting's rate, which we can't predict at forecast time).
    /// Returns 1.0 when settings are disabled.
    fn settings_forecast_multiplier(&self, person: PersonId) -> f64;

    /// Pick a `Setting` for `person` weighted by `p_s · M_s`. Returns `None`
    /// when settings are disabled or every weight is zero.
    fn settings_sample_current_setting(&self, person: PersonId) -> Option<SettingId>;

    /// Pick a contact for `infector`. When settings are enabled, samples
    /// a setting via `settings_sample_current_setting` then a member of
    /// that setting (excluding the infector). When disabled, samples
    /// globally — preserving the old random-mixing behavior.
    fn settings_sample_contact(&self, infector: PersonId) -> Option<PersonId>;

    /// Restrict `person`'s current itinerary to their setting of type
    /// `type_index` (isolation). While restricted,
    /// `settings_current_multiplier` and contact sampling use only that
    /// setting; the forecast bound (`max_s M_s`, used because restriction is
    /// armed — see `settings_forecast_multiplier`) is unaffected, so
    /// `current ≤ forecasted` still holds. No-op when settings are disabled
    /// or `person` has no setting of that type.
    fn settings_restrict_to(&mut self, person: PersonId, type_index: usize);

    /// Remove any itinerary restriction on `person`, restoring the full
    /// itinerary. No-op if `person` wasn't restricted.
    fn settings_clear_restriction(&mut self, person: PersonId);

    /// Whether `person` currently has an itinerary restriction.
    fn settings_is_restricted(&self, person: PersonId) -> bool;
}

impl SettingsExt for Context {
    fn settings_setup(&mut self, types: &[SettingType]) {
        if types.is_empty() {
            return;
        }
        // Store the type list; everything downstream reads through here.
        self.get_data_mut(SettingsPlugin).types = types.to_vec();

        let n_types = types.len();
        let people: Vec<PersonId> = self.query_result_iterator(Person).collect();
        let pop_size = people.len();
        if pop_size == 0 {
            return;
        }

        // Pre-seed each person's settings vector so per-type pushes are
        // O(1). The Vecs grow to len = n_types as we fill each type.
        {
            let map = &mut self.get_data_mut(SettingsPlugin).person_to_settings;
            for &p in &people {
                map.insert(p, Vec::with_capacity(n_types));
            }
        }

        for (type_idx, st) in types.iter().enumerate() {
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
                let mut size = size_values[size_idx];
                // Last setting may be truncated to whatever's left.
                if cursor + size > pop_size {
                    size = pop_size - cursor;
                }

                let setting_id: SettingId = self.add_entity(Setting).unwrap();
                self.set_property(setting_id, SettingTypeIndex(type_idx));

                let members: Vec<PersonId> = shuffled[cursor..cursor + size].to_vec();
                // Realized size may differ from the sampled size for the
                // last setting; recompute the multiplier from `members.len()`.
                let m_s = st.multiplier(members.len());
                {
                    let plugin = self.get_data_mut(SettingsPlugin);
                    for &p in &members {
                        plugin
                            .person_to_settings
                            .get_mut(p)
                            .unwrap()
                            .push(setting_id);
                    }
                    plugin.members.insert(setting_id, members);
                    plugin.setting_multiplier.insert(setting_id, m_s);
                }

                cursor += size;
            }
        }

        // Precompute per-person scalars and the per-person selection-weight
        // vector. Membership is static so these only need computing once.
        let mut precomputed: Vec<(PersonId, f64, f64, Vec<f64>)> = Vec::with_capacity(pop_size);
        {
            let data = self.get_data(SettingsPlugin);
            for &p in &people {
                let Some(settings) = data.person_to_settings.get(p) else {
                    precomputed.push((p, 0.0, 0.0, Vec::new()));
                    continue;
                };
                let mut m_max = 0.0_f64;
                let mut m_sum = 0.0_f64;
                let mut weights: Vec<f64> = Vec::with_capacity(settings.len());
                for (ti, &sid) in settings.iter().enumerate() {
                    let m = *data.setting_multiplier.get(sid).unwrap_or(&0.0);
                    let prop = data.types[ti].proportion;
                    if m > m_max {
                        m_max = m;
                    }
                    m_sum += prop * m;
                    weights.push(prop * m);
                }
                precomputed.push((p, m_max, m_sum, weights));
            }
        }
        let plugin = self.get_data_mut(SettingsPlugin);
        for (p, m_max, m_sum, weights) in precomputed {
            plugin.max_multiplier.insert(p, m_max);
            plugin.current_multiplier.insert(p, m_sum);
            plugin.selection_weights.insert(p, weights);
        }
    }

    fn settings_max_multiplier(&self, person: PersonId) -> f64 {
        let data = self.get_data(SettingsPlugin);
        if !data.enabled() {
            return 1.0;
        }
        data.max_multiplier.get(person).copied().unwrap_or(0.0)
    }

    fn settings_current_multiplier(&self, person: PersonId) -> f64 {
        let data = self.get_data(SettingsPlugin);
        if !data.enabled() {
            return 1.0;
        }
        // Isolation: confined to one setting → just that setting's M_target
        // (≤ max_multiplier, so the thinning invariant holds).
        if let Some(&sid) = data.restriction.get(person) {
            return data.setting_multiplier.get(sid).copied().unwrap_or(0.0);
        }
        data.current_multiplier.get(person).copied().unwrap_or(0.0)
    }

    fn settings_arm_restriction(&mut self) {
        self.get_data_mut(SettingsPlugin).restriction_armed = true;
    }

    fn settings_forecast_multiplier(&self, person: PersonId) -> f64 {
        // No restriction possible → current_mult is the exact, constant total
        // rate (membership is static), so it's a tight forecast bound and the
        // thinning step accepts every event. Restriction armed → widen to the
        // max, since a restricted person's rate can exceed their weighted mean.
        if self.get_data(SettingsPlugin).restriction_armed {
            self.settings_max_multiplier(person)
        } else {
            self.settings_current_multiplier(person)
        }
    }

    fn settings_sample_current_setting(&self, person: PersonId) -> Option<SettingId> {
        let data = self.get_data(SettingsPlugin);
        if !data.enabled() {
            return None;
        }
        // Isolation: contacts come only from the restricted setting.
        if let Some(&sid) = data.restriction.get(person) {
            return Some(sid);
        }
        let settings = data.person_to_settings.get(person)?;
        let weights = data.selection_weights.get(person)?;
        if weights.iter().all(|w| *w <= 0.0) {
            return None;
        }
        let idx = self.sample_weighted(SettingsRng, weights);
        Some(settings[idx])
    }

    fn settings_sample_contact(&self, infector: PersonId) -> Option<PersonId> {
        let data = self.get_data(SettingsPlugin);
        if !data.enabled() {
            // Global random mixing.
            let q = self.query(Person);
            return self.sample(SettingsContactRng, move |rng| {
                q.sample_entity_excluding(rng, infector)
            });
        }

        let setting_id = self.settings_sample_current_setting(infector)?;

        let members = data.members.get(setting_id)?;
        self.sample(SettingsContactRng, |rng| {
            sample_single_excluding(rng, members, infector).copied()
        })
    }

    fn settings_restrict_to(&mut self, person: PersonId, type_index: usize) {
        let data = self.get_data(SettingsPlugin);
        if !data.enabled() {
            return;
        }
        let Some(&sid) = data
            .person_to_settings
            .get(person)
            .and_then(|settings| settings.get(type_index))
        else {
            return;
        };
        self.get_data_mut(SettingsPlugin)
            .restriction
            .insert(person, sid);
    }

    fn settings_clear_restriction(&mut self, person: PersonId) {
        let _ = self.get_data_mut(SettingsPlugin).restriction.remove(person);
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

    #[test]
    fn disabled_returns_neutral_values() {
        let ctx = build_context_with(Vec::new(), 5, 0);
        let people: Vec<PersonId> = ctx.query_result_iterator(Person).collect();
        for p in people {
            assert_eq!(ctx.settings_max_multiplier(p), 1.0);
            assert_eq!(ctx.settings_current_multiplier(p), 1.0);
            assert_eq!(ctx.settings_forecast_multiplier(p), 1.0);
        }
    }

    #[test]
    fn disabled_sample_contact_excludes_infector() {
        // Without settings, contact selection should still avoid self-pick
        // (matches the prior global-mixing behavior).
        let ctx = build_context_with(Vec::new(), 4, 1);
        let people: Vec<PersonId> = ctx.query_result_iterator(Person).collect();
        let infector = people[0];
        for _ in 0..200 {
            let pick = ctx.settings_sample_contact(infector).unwrap();
            assert_ne!(pick, infector);
        }
    }

    #[test]
    fn setup_assigns_everyone_one_setting_per_type() {
        // Two types, deterministic size = 3.
        let t1 = make_type("home", 1.0, vec![(3, 1.0)], 0.5);
        let t2 = make_type("work", 0.5, vec![(3, 1.0)], 0.5);
        let ctx = build_context_with(vec![t1, t2], 9, 0);
        let data = ctx.get_data(SettingsPlugin);
        for p in ctx.query_result_iterator(Person) {
            let s = data.person_to_settings.get(p).expect("person has settings");
            assert_eq!(
                s.len(),
                2,
                "expected 2 setting assignments, got {}",
                s.len()
            );
        }
        // 9 people / size 3 = 3 settings per type, 6 total.
        let mut count_by_type = [0usize; 2];
        for (sid, _) in data.members.iter() {
            let ti = ctx.get_property::<_, SettingTypeIndex>(sid).0;
            count_by_type[ti] += 1;
        }
        assert_eq!(count_by_type, [3, 3]);
    }

    #[test]
    fn setup_handles_undersized_last_setting() {
        // Size 4 with population 9 → settings of size [4, 4, 1].
        let t = make_type("home", 1.0, vec![(4, 1.0)], 1.0);
        let ctx = build_context_with(vec![t], 9, 0);
        let data = ctx.get_data(SettingsPlugin);
        let mut sizes: Vec<usize> = data.members.iter().map(|(_, m)| m.len()).collect();
        sizes.sort_unstable();
        assert_eq!(sizes, vec![1, 4, 4]);
    }

    #[test]
    fn multipliers_match_expected_for_known_assignment() {
        // Single type, single size (4), alpha=1. Per-person multiplier
        // M = (4-1)^1 = 3. proportion = 1.0 → current = 3, max = 3.
        let t = make_type("home", 1.0, vec![(4, 1.0)], 1.0);
        let ctx = build_context_with(vec![t], 8, 0);
        for p in ctx.query_result_iterator(Person) {
            assert_almost_eq!(ctx.settings_max_multiplier(p), 3.0, 1e-12);
            assert_almost_eq!(ctx.settings_current_multiplier(p), 3.0, 1e-12);
        }
    }

    #[test]
    fn current_is_proportion_weighted_sum() {
        // Two types: A (size 5, alpha=1, p=0.3) → M=4; B (size 3, alpha=1, p=0.7) → M=2.
        // Expected current = 0.3·4 + 0.7·2 = 2.6. Max = max(4, 2) = 4.
        let a = make_type("a", 1.0, vec![(5, 1.0)], 0.3);
        let b = make_type("b", 1.0, vec![(3, 1.0)], 0.7);
        let ctx = build_context_with(vec![a, b], 15, 0);
        for p in ctx.query_result_iterator(Person) {
            assert_almost_eq!(ctx.settings_current_multiplier(p), 2.6, 1e-9);
            assert_almost_eq!(ctx.settings_max_multiplier(p), 4.0, 1e-9);
        }
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn sample_contact_picks_only_setting_members_excluding_self() {
        // Two settings of size 3 of the same type; we know which people are
        // in each by inspecting the plugin. Sampling for a member of setting
        // X should only ever return members of X (not the other setting).
        let t = make_type("home", 1.0, vec![(3, 1.0)], 1.0);
        let ctx = build_context_with(vec![t], 6, 7);
        let data = ctx.get_data(SettingsPlugin);
        let p0 = ctx.query_result_iterator(Person).next().unwrap();
        let p0_setting = data.person_to_settings.get(p0).unwrap()[0];
        let mut allowed: std::collections::HashSet<PersonId> = data
            .members
            .get(p0_setting)
            .unwrap()
            .iter()
            .copied()
            .collect();
        allowed.remove(&p0);
        // 200 draws — large enough to hit every allowed member.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let pick = ctx.settings_sample_contact(p0).expect("expected a contact");
            assert!(
                allowed.contains(&pick),
                "pick {pick:?} not in setting members"
            );
            seen.insert(pick);
        }
        assert_eq!(seen.len(), allowed.len(), "did not exercise all members");
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn sample_current_setting_distribution_matches_p_times_m() {
        // Two types with different M_s and proportions. Setting-selection
        // probability should approach (p_s · M_s) / Σ p_s · M_s.
        //   A: size 5 (M=4), proportion 0.5  → weight 2.0
        //   B: size 2 (M=1), proportion 0.5  → weight 0.5
        //   P(A) → 0.8, P(B) → 0.2.
        let a = make_type("a", 1.0, vec![(5, 1.0)], 0.5);
        let b = make_type("b", 1.0, vec![(2, 1.0)], 0.5);
        let ctx = build_context_with(vec![a, b], 10, 0);
        let p = ctx.query_result_iterator(Person).next().unwrap();

        let mut counts = [0usize; 2];
        for _ in 0..20_000 {
            let sid = ctx.settings_sample_current_setting(p).unwrap();
            let ti = ctx.get_property::<_, SettingTypeIndex>(sid).0;
            counts[ti] += 1;
        }
        let total = (counts[0] + counts[1]) as f64;
        let p_a = counts[0] as f64 / total;
        // 20k Bernoulli(0.8) samples; ±0.02 is generous (~7σ).
        assert!(
            (p_a - 0.8).abs() < 0.02,
            "expected P(A) ≈ 0.8, observed {p_a} (counts {counts:?})"
        );
    }

    #[test]
    fn restrict_to_uses_target_multiplier_and_leaves_max() {
        // home: size 3, alpha 1 → M=2; work: size 5, alpha 1 → M=4.
        // proportions 0.5/0.5 → current = 0.5·2 + 0.5·4 = 3; max = max(2,4) = 4.
        let home = make_type("home", 1.0, vec![(3, 1.0)], 0.5);
        let work = make_type("work", 1.0, vec![(5, 1.0)], 0.5);
        let mut ctx = build_context_with(vec![home, work], 15, 0);
        let p = ctx.query_result_iterator(Person).next().unwrap();

        assert_almost_eq!(ctx.settings_current_multiplier(p), 3.0, 1e-9);
        assert_almost_eq!(ctx.settings_max_multiplier(p), 4.0, 1e-9);

        // Restrict to home (type index 0): current → M_home = 2; max unchanged.
        ctx.settings_restrict_to(p, 0);
        assert!(ctx.settings_is_restricted(p));
        assert_almost_eq!(ctx.settings_current_multiplier(p), 2.0, 1e-9);
        assert_almost_eq!(ctx.settings_max_multiplier(p), 4.0, 1e-9);

        // Clearing restores the full weighted current.
        ctx.settings_clear_restriction(p);
        assert!(!ctx.settings_is_restricted(p));
        assert_almost_eq!(ctx.settings_current_multiplier(p), 3.0, 1e-9);
    }

    #[test]
    fn forecast_multiplier_is_tight_until_restriction_armed() {
        // Same home/work setup: current = 3, max = 4 (current strictly < max).
        let home = make_type("home", 1.0, vec![(3, 1.0)], 0.5);
        let work = make_type("work", 1.0, vec![(5, 1.0)], 0.5);
        let mut ctx = build_context_with(vec![home, work], 15, 0);
        let p = ctx.query_result_iterator(Person).next().unwrap();

        // Unarmed: the forecast bound is the *exact* total rate (current_mult),
        // strictly below max — this is what makes the thinning step a no-op.
        assert_almost_eq!(ctx.settings_forecast_multiplier(p), 3.0, 1e-9);
        assert_almost_eq!(
            ctx.settings_forecast_multiplier(p),
            ctx.settings_current_multiplier(p),
            1e-12
        );

        // Arming (what isolation::register does) widens the bound to max_s M_s,
        // so a restriction raising a person's rate up to a peak setting stays
        // ≤ the forecast bound and `current ≤ forecasted` holds.
        ctx.settings_arm_restriction();
        assert_almost_eq!(ctx.settings_forecast_multiplier(p), 4.0, 1e-9);
        assert_almost_eq!(
            ctx.settings_forecast_multiplier(p),
            ctx.settings_max_multiplier(p),
            1e-12
        );
    }

    #[test]
    fn restricted_sampling_only_hits_target_setting() {
        let home = make_type("home", 1.0, vec![(3, 1.0)], 0.5);
        let work = make_type("work", 1.0, vec![(5, 1.0)], 0.5);
        let mut ctx = build_context_with(vec![home, work], 15, 7);
        let p = ctx.query_result_iterator(Person).next().unwrap();
        ctx.settings_restrict_to(p, 0); // home

        let data = ctx.get_data(SettingsPlugin);
        let home_sid = data.person_to_settings.get(p).unwrap()[0];
        let home_members: std::collections::HashSet<PersonId> = data
            .members
            .get(home_sid)
            .unwrap()
            .iter()
            .copied()
            .collect();

        // Every sampled contact must be a home member other than the infector.
        for _ in 0..200 {
            let c = ctx
                .settings_sample_contact(p)
                .expect("home setting has other members");
            assert_ne!(c, p);
            assert!(home_members.contains(&c), "contact {c:?} not in home");
        }
    }

    #[test]
    fn restrict_is_noop_when_disabled() {
        let mut ctx = build_context_with(Vec::new(), 5, 0);
        let p = ctx.query_result_iterator(Person).next().unwrap();
        ctx.settings_restrict_to(p, 0);
        assert!(!ctx.settings_is_restricted(p));
        // Disabled settings still report the neutral 1.0.
        assert_eq!(ctx.settings_current_multiplier(p), 1.0);
    }
}
