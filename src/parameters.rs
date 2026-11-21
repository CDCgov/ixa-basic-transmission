use ixa::prelude::*;
use ixa::{define_global_property, Context};
use serde::{Deserialize, Serialize};

use crate::modifiers::Modifiers;
use crate::rate::InfectionRate;
use crate::settings::SettingType;

define_global_property!(Params, Parameters);

/// The parameters of the run, set once before setup.
pub trait ParametersExt {
    /// # Panics
    ///
    /// If they were already set.
    fn set_params(&mut self, params: Parameters);

    /// # Panics
    ///
    /// If they were not set.
    fn params(&self) -> &Parameters;
}

impl ParametersExt for Context {
    fn set_params(&mut self, params: Parameters) {
        self.set_global_property_value(Params, params)
            .expect("parameters were already set");
    }

    fn params(&self) -> &Parameters {
        self.get_global_property_value(Params)
            .expect("parameters were not set")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Parameters {
    // Per-person infectiousness profile. The `Constant` variant bundles
    // its own `exp_mean_duration` (`duration` on the wire); the `Empirical`
    // variant derives the period from its curve's support.
    pub infection_rate: InfectionRate,
    pub population: usize,
    pub initial_infections: usize,
    pub seed: u64,
    pub max_time: f64,
    /// Optional settings-based contact structure. Empty (the default)
    /// means the model uses global random mixing — same as before this
    /// feature was added. See `crate::settings`.
    #[serde(default)]
    pub settings: Vec<SettingType>,
    /// Transmission-modifier interventions (facemask, antiviral, isolation),
    /// grouped as one subsection. Each is disabled by default. See
    /// `crate::modifiers`.
    #[serde(default)]
    pub modifiers: Modifiers,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            infection_rate: InfectionRate::Constant {
                value: 0.5,
                exp_mean_duration: 3.0,
            },
            population: 10000,
            initial_infections: 5,
            seed: 0,
            max_time: 100.0,
            settings: Vec::new(),
            modifiers: Modifiers::default(),
        }
    }
}

impl Parameters {
    pub fn validate(&self) -> Result<(), String> {
        self.infection_rate.validate()?;
        if !self.max_time.is_finite() || self.max_time < 0.0 {
            return Err(format!(
                "max_time must be a finite non-negative number, got {}",
                self.max_time
            ));
        }
        if self.initial_infections > self.population {
            return Err(format!(
                "initial_infections ({}) must not exceed population ({})",
                self.initial_infections, self.population
            ));
        }
        if !self.settings.is_empty() {
            if self.settings.len() > ixa::core_epi::MAX_SETTINGS {
                return Err(format!(
                    "at most {} setting types are supported, got {}",
                    ixa::core_epi::MAX_SETTINGS,
                    self.settings.len()
                ));
            }
            for s in &self.settings {
                s.validate()?;
            }
            for (i, s) in self.settings.iter().enumerate() {
                if self.settings[..i].iter().any(|other| other.name == s.name) {
                    return Err(format!("setting name {} is used more than once", s.name));
                }
            }
            let total: f64 = self.settings.iter().map(|s| s.proportion).sum();
            if total <= 0.0 || !total.is_finite() {
                return Err(format!(
                    "settings proportions must sum to a positive finite number, got {total}"
                ));
            }
        }
        self.modifiers.validate()?;
        if let Some(isolation) = &self.modifiers.isolation {
            let target = &isolation.restrict_to;
            if !self.settings.iter().any(|setting| &setting.name == target) {
                return Err(format!(
                    "isolation restrictTo must name a configured setting, got {target}"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_valid() {
        Parameters::default().validate().unwrap();
    }

    #[test]
    fn isolation_must_restrict_to_a_configured_setting() {
        let isolation = |name: &str| crate::modifiers::isolation::Isolation {
            coverage: 1.0,
            restrict_to: name.into(),
            delay: 0.0,
            duration: None,
        };
        let with = |name: &str, settings: Vec<crate::settings::SettingType>| Parameters {
            settings,
            modifiers: Modifiers {
                isolation: Some(isolation(name)),
                ..Default::default()
            },
            ..Default::default()
        };
        let household = crate::settings::SettingType {
            name: "household".into(),
            alpha: 1.0,
            sizes: vec![(3, 1.0)],
            proportion: 1.0,
        };
        with("household", vec![household.clone()])
            .validate()
            .unwrap();
        assert!(with("spaceship", vec![household]).validate().is_err());
        assert!(with("household", vec![]).validate().is_err());
    }

    #[test]
    fn validate_delegates_to_rate() {
        // Sanity: an invalid rate makes Parameters invalid too. Detailed
        // rate-validation cases live in `rate::tests`.
        let p = Parameters {
            infection_rate: InfectionRate::Empirical {
                points: vec![],
                scale: 1.0,
            },
            ..Parameters::default()
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_negative_max_time() {
        let p = Parameters {
            max_time: -1.0,
            ..Parameters::default()
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_initial_infections_exceeding_population() {
        let p = Parameters {
            population: 10,
            initial_infections: 11,
            ..Parameters::default()
        };
        assert!(p.validate().is_err());
    }
}
