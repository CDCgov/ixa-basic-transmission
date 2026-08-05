// Ports `def_abc_smc/src/dist.rs`. Same `Distribution<T>` trait shape;
// k88 chose Uniform (instead of def's Exp) for the r0 prior, so the
// `Exp` distribution is omitted here.

use rand::{Rng, RngExt};

pub trait Distribution<T> {
    fn sample(&self, rng: &mut impl Rng) -> T;
    // pdf for continuous, pmf for discrete.
    fn weight(&self, value: T) -> f64;
}

/// Continuous uniform on `[lower, upper)`.
pub struct Uniform {
    lower: f64,
    upper: f64,
}

impl Uniform {
    pub fn new(lower: f64, upper: f64) -> Self {
        assert!(
            upper > lower,
            "Uniform::new requires upper > lower (got {lower}, {upper})"
        );
        Self { lower, upper }
    }

    pub fn lower(&self) -> f64 {
        self.lower
    }
    pub fn upper(&self) -> f64 {
        self.upper
    }
}

impl Distribution<f64> for Uniform {
    fn sample(&self, rng: &mut impl Rng) -> f64 {
        rng.random_range(self.lower..self.upper)
    }

    fn weight(&self, value: f64) -> f64 {
        if value >= self.lower && value < self.upper {
            1.0 / (self.upper - self.lower)
        } else {
            0.0
        }
    }
}

const FRAC_1_SQRT_2PI: f64 = 0.398_942_280_401_432_7_f64;

pub struct Normal {
    inner_dist: rand_distr::Normal<f64>,
}

impl Normal {
    pub fn new(mean: f64, sd: f64) -> Self {
        Self {
            inner_dist: rand_distr::Normal::new(mean, sd).unwrap(),
        }
    }
}

impl Distribution<f64> for Normal {
    fn sample(&self, rng: &mut impl Rng) -> f64 {
        rng.sample(self.inner_dist)
    }

    fn weight(&self, value: f64) -> f64 {
        let mean = self.inner_dist.mean();
        let sd = self.inner_dist.std_dev();
        let d = (value - mean) / sd;
        (-0.5 * d * d).exp() * FRAC_1_SQRT_2PI / sd
    }
}

pub struct DiscreteUniform<T> {
    lower: T,
    // Exclusive upper bound (matches def).
    upper: T,
}

impl<T> DiscreteUniform<T> {
    pub fn new(lower: T, upper: T) -> Self {
        Self { lower, upper }
    }
}

impl<T: Copy> DiscreteUniform<T> {
    pub fn lower(&self) -> T {
        self.lower
    }
    pub fn upper(&self) -> T {
        self.upper
    }
}

impl Distribution<u64> for DiscreteUniform<u64> {
    fn sample(&self, rng: &mut impl Rng) -> u64 {
        rng.random_range(self.lower..self.upper)
    }

    fn weight(&self, value: u64) -> f64 {
        if value < self.upper && value >= self.lower {
            1.0 / (self.upper - self.lower) as f64
        } else {
            0.0
        }
    }
}

impl Distribution<i64> for DiscreteUniform<i64> {
    fn sample(&self, rng: &mut impl Rng) -> i64 {
        rng.random_range(self.lower..self.upper)
    }

    fn weight(&self, value: i64) -> f64 {
        if value < self.upper && value >= self.lower {
            1.0 / (self.upper - self.lower) as f64
        } else {
            0.0
        }
    }
}
