use super::InfectiousnessRateFn;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug)]
pub struct RateFnError(pub String);

impl fmt::Display for RateFnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for RateFnError {}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConstantRateParams {
    // A rate of infection in terms of people per unit time
    pub r: f64,
    // The time after which the rate of infection becomes 0
    pub infection_duration: f64,
}

pub struct ConstantRate {
    parameters: ConstantRateParams,
}

impl ConstantRate {
    pub fn new(rate: f64, infection_duration: f64) -> Result<Self, RateFnError> {
        ConstantRate::try_from(ConstantRateParams {
            r: rate,
            infection_duration,
        })
    }
}

impl TryFrom<ConstantRateParams> for ConstantRate {
    type Error = RateFnError;
    fn try_from(parameters: ConstantRateParams) -> Result<Self, RateFnError> {
        if parameters.r < 0.0 {
            return Err(RateFnError(
                "The rate of infection must be non-negative.".to_string(),
            ));
        }
        if parameters.infection_duration < 0.0 {
            return Err(RateFnError(
                "The duration of infection must be non-negative.".to_string(),
            ));
        }
        Ok(Self { parameters })
    }
}

impl InfectiousnessRateFn for ConstantRate {
    fn rate(&self, t: f64) -> f64 {
        if t > self.parameters.infection_duration {
            return 0.0;
        }
        self.parameters.r
    }
    fn cum_rate(&self, t: f64) -> f64 {
        self.parameters.r * t.min(self.parameters.infection_duration)
    }
    fn inverse_cum_rate(&self, events: f64) -> Option<f64> {
        let t = events / self.parameters.r;
        if t > self.parameters.infection_duration {
            None
        } else {
            Some(t)
        }
    }
    fn infection_duration(&self) -> f64 {
        self.parameters.infection_duration
    }
}

#[cfg(test)]
mod test {
    use approx::assert_abs_diff_eq;

    use super::ConstantRate;
    use super::InfectiousnessRateFn;
    use super::RateFnError;

    #[test]
    fn test_constant_rate_errors_r_negative() {
        let e = ConstantRate::new(-1.0, 10.0).err();
        match e {
            Some(RateFnError(msg)) => {
                assert_eq!(
                    msg,
                    "The rate of infection must be non-negative.".to_string()
                );
            }
            None => {
                panic!("Expected an error. Instead, created a constant rate struct with no errors.")
            }
        }
    }

    #[test]
    fn test_constant_rate_errors_infection_duration_negative() {
        let e = ConstantRate::new(1.0, -1.0).err();
        match e {
            Some(RateFnError(msg)) => {
                assert_eq!(
                    msg,
                    "The duration of infection must be non-negative.".to_string()
                );
            }
            None => {
                panic!("Expected an error. Instead, created a constant rate struct with no errors.")
            }
        }
    }

    #[test]
    fn test_rate() {
        let r = ConstantRate::new(2.0, 10.0).unwrap();
        assert_abs_diff_eq!(r.rate(5.0), 2.0);
        assert_abs_diff_eq!(r.rate(11.0), 0.0);
    }

    #[test]
    fn test_cum_rate() {
        let r = ConstantRate::new(2.0, 10.0).unwrap();
        assert_abs_diff_eq!(r.cum_rate(5.0), 10.0);
        assert_abs_diff_eq!(r.cum_rate(11.0), 20.0);
    }

    #[test]
    fn test_inverse_cum_rate() {
        let r = ConstantRate::new(2.0, 10.0).unwrap();
        assert_eq!(r.inverse_cum_rate(10.0), Some(5.0));
        assert_eq!(r.inverse_cum_rate(20.0), Some(10.0));
        assert_eq!(r.inverse_cum_rate(21.0), None);
    }

    #[test]
    fn test_infection_duration() {
        let r = ConstantRate::new(2.0, 10.0).unwrap();
        assert_abs_diff_eq!(r.infection_duration(), 10.0);
    }
}
