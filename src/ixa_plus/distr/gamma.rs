pub use super::distribution::{ContinuousUnivariate, Distribution};
pub use crate::gamma;
use serde::{Deserialize, Serialize};
use statrs::{
    distribution::{self as sd, Continuous, ContinuousCDF},
    statistics::Distribution as StatisticsDistribution,
};

#[macro_export]
macro_rules! gamma {
    (shape = $shape:expr, scale = $scale:expr) => {
        $crate::ixa_plus::distr::gamma::GammaParams::Scale {
            shape: $shape,
            scale: $scale,
        }
        .try_into()
    };
    (shape = $shape:expr, rate = $rate:expr) => {
        $crate::ixa_plus::distr::gamma::GammaParams::Rate {
            shape: $shape,
            rate: $rate,
        }
        .try_into()
    };
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
#[serde(untagged)]
pub enum GammaParams {
    Rate { shape: f64, rate: f64 },
    Scale { shape: f64, scale: f64 },
}

impl TryInto<sd::Gamma> for GammaParams {
    type Error = sd::GammaError;
    fn try_into(self) -> Result<sd::Gamma, sd::GammaError> {
        match self {
            GammaParams::Rate { shape, rate } => sd::Gamma::new(shape, 1.0 / rate),
            GammaParams::Scale { shape, scale } => sd::Gamma::new(shape, scale),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Gamma {
    params: GammaParams,
    distr: sd::Gamma,
}

impl Gamma {
    pub fn from_shape_rate(shape: f64, rate: f64) -> Result<Self, sd::GammaError> {
        let params = GammaParams::Rate { shape, rate };
        Ok(Self {
            params,
            distr: params.clone().try_into()?,
        })
    }
    pub fn from_shape_scale(shape: f64, scale: f64) -> Result<Self, sd::GammaError> {
        let params = GammaParams::Scale { shape, scale };
        Ok(Self {
            params,
            distr: params.clone().try_into()?,
        })
    }
    pub fn rate(&self) -> f64 {
        match self.params {
            GammaParams::Rate { rate, .. } => rate,
            GammaParams::Scale { scale, .. } => 1.0 / scale,
        }
    }
    pub fn scale(&self) -> f64 {
        match self.params {
            GammaParams::Rate { rate, .. } => 1.0 / rate,
            GammaParams::Scale { scale, .. } => scale,
        }
    }
    pub fn mean(&self) -> f64 {
        self.distr.mean().unwrap()
    }
}

// Mirror trait methods for convenience
impl Gamma {
    pub fn sample<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> f64 {
        <Self as Distribution<f64>>::sample(self, rng)
    }
}

impl TryFrom<GammaParams> for Gamma {
    type Error = sd::GammaError;

    fn try_from(params: GammaParams) -> Result<Self, Self::Error> {
        match params {
            GammaParams::Rate { shape, rate } => Self::from_shape_rate(shape, rate),
            GammaParams::Scale { shape, scale } => Self::from_shape_scale(shape, scale),
        }
    }
}

impl Serialize for Gamma {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.params.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Gamma {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let params = GammaParams::deserialize(deserializer)?;
        let gamma = params.try_into().map_err(serde::de::Error::custom)?;
        Ok(gamma)
    }
}

impl ContinuousUnivariate<f64, f64> for Gamma {
    fn pdf(&self, x: f64) -> f64 {
        self.distr.pdf(x)
    }
    fn ln_pdf(&self, x: f64) -> f64 {
        self.distr.ln_pdf(x)
    }
    fn cdf(&self, x: f64) -> f64 {
        self.distr.cdf(x)
    }
    fn inverse_cdf(&self, p: f64) -> f64 {
        self.distr.inverse_cdf(p)
    }
}

impl Distribution<f64> for Gamma {
    fn sample<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> f64 {
        self.distr.sample(rng)
    }
}
