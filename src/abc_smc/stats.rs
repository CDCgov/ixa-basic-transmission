// Direct port of `def_abc_smc/src/stats.rs`. Welford 1962.

pub struct MeanVarianceEstimator {
    mean: f64,
    sum_squares: f64,
    n_samples: u64,
}

impl Default for MeanVarianceEstimator {
    fn default() -> Self {
        Self::new()
    }
}

impl MeanVarianceEstimator {
    pub fn new() -> Self {
        Self {
            mean: 0.0,
            sum_squares: 0.0,
            n_samples: 0,
        }
    }

    pub fn add(&mut self, sample: f64) {
        let prev_mean = self.mean;
        let prev_n_samples = self.n_samples;
        self.n_samples += 1;
        self.mean += (sample - self.mean) / self.n_samples as f64;
        self.sum_squares += (prev_n_samples as f64 / self.n_samples as f64)
            * (sample - prev_mean)
            * (sample - prev_mean);
    }

    pub fn n_samples(&self) -> u64 {
        self.n_samples
    }

    // Caller must ensure at least one sample.
    pub fn get_mean(&self) -> f64 {
        self.mean
    }

    // Caller must ensure at least two samples.
    pub fn get_variance(&self) -> f64 {
        self.sum_squares / (self.n_samples - 1) as f64
    }
}

#[cfg(test)]
mod test {
    use super::MeanVarianceEstimator;
    use rand::{rngs::StdRng, Rng, SeedableRng};
    use rand_distr::Normal;

    // Ported verbatim from def_abc_smc/src/stats.rs::test::test_normal.
    #[test]
    fn test_normal() {
        let n_samples = 100_000;
        let mean = 1.0;
        let sd = 2.0;
        let normal = Normal::new(mean, sd).unwrap();
        let mut rng = StdRng::seed_from_u64(8675309);

        let mut estimator = MeanVarianceEstimator::new();
        for _ in 0..n_samples {
            estimator.add(rng.sample(normal));
        }

        assert!(f64::abs(estimator.get_mean() - mean) < 1e-3);
        assert!(f64::abs(estimator.get_variance().sqrt() - sd) < 1e-2);
    }
}
