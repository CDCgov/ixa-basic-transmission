// Adapts `def_abc_smc/src/particle.rs`. `model_output` is the existing
// `crate::stats::ModelStats` from a single `crate::model::run` rather
// than def's `RenewalOutput`.

use super::priors::CalibratedParams;
use crate::stats::ModelStats;

#[derive(Clone)]
pub struct Particle {
    pub parameters: CalibratedParams,
    pub model_output: ModelStats,
    pub data_distance: u64,
    pub weight: f64,
}
