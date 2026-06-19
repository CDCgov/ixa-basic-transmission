use serde::{Deserialize, Serialize};

use crate::modifiers::antiviral::Antiviral;
use crate::modifiers::facemask::Facemask;
use crate::rate::InfectionRate;
use crate::settings::SettingType;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Parameters {
    // Per-person infectiousness profile. The `Constant` variant bundles
    // its own `duration` (mean of the `Exp` recovery); the `Empirical`
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
    /// Optional facemask intervention. `None` (the default) disables it
    /// entirely — no RNG is drawn and infectiousness is unmodified. See
    /// `crate::modifiers::facemask`.
    #[serde(default)]
    pub facemask: Option<Facemask>,
    /// Optional antiviral-treatment intervention. `None` (the default)
    /// disables it entirely. See `crate::modifiers::antiviral`.
    #[serde(default)]
    pub antiviral: Option<Antiviral>,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            infection_rate: InfectionRate::Constant {
                value: 0.5,
                duration: 3.0,
            },
            population: 10000,
            initial_infections: 5,
            seed: 0,
            max_time: 100.0,
            settings: Vec::new(),
            facemask: None,
            antiviral: None,
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
            for s in &self.settings {
                s.validate()?;
            }
            let total: f64 = self.settings.iter().map(|s| s.proportion).sum();
            if total <= 0.0 || !total.is_finite() {
                return Err(format!(
                    "settings proportions must sum to a positive finite number, got {total}"
                ));
            }
        }
        if let Some(facemask) = &self.facemask {
            facemask.validate()?;
        }
        if let Some(antiviral) = &self.antiviral {
            antiviral.validate()?;
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
