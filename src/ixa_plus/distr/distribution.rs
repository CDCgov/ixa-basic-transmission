pub use rand::distr::Distribution;
pub trait ContinuousUnivariate<K, T> {
    fn pdf(&self, x: K) -> T;
    fn ln_pdf(&self, x: K) -> T;
    fn cdf(&self, x: K) -> T;
    fn inverse_cdf(&self, p: T) -> K;
}
