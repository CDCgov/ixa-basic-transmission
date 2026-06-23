use super::InfectiousnessRateFn;

/// A time-invariant infectiousness rate (a homogeneous Poisson hazard). `value` is an
/// absolute hazard rate that applies for all time — the rate is **not** capped by a
/// duration. Recovery in this model is an independent `Exp(1/duration)` clock owned by
/// the transmission loop (see `model::schedule_recovery`); the `duration` lives on the
/// `InfectionRate::Constant` config, not here.
///
/// Because the rate never falls to zero, the cumulative is unbounded and
/// `inverse_cum_rate` never returns `None` for a positive rate — consistent with a
/// homogeneous Poisson process that only stops when the recovery clock fires.
#[derive(Debug, Clone, Copy)]
pub struct ConstantRate {
    value: f64,
}

impl ConstantRate {
    #[must_use]
    pub fn new(value: f64) -> Self {
        Self { value }
    }
}

impl InfectiousnessRateFn for ConstantRate {
    #[inline]
    fn rate(&self, _t: f64) -> f64 {
        self.value
    }

    #[inline]
    fn cum_rate(&self, t: f64) -> f64 {
        if t > 0.0 {
            self.value * t
        } else {
            0.0
        }
    }

    #[inline]
    fn inverse_cum_rate(&self, events: f64) -> Option<f64> {
        if events < 0.0 {
            return None;
        }
        if events == 0.0 {
            return Some(0.0);
        }
        if !self.value.is_finite() || self.value <= 0.0 {
            None
        } else {
            Some(events / self.value)
        }
    }

    #[inline]
    fn max_cum_rate(&self) -> f64 {
        // A positive constant hazard integrates to infinity; a zero rate exhausts
        // immediately (no events ever).
        if self.value > 0.0 {
            f64::INFINITY
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod test {
    use super::ConstantRate;
    use super::InfectiousnessRateFn;

    #[test]
    fn rate_is_time_invariant() {
        let r = ConstantRate::new(0.7);
        assert_eq!(r.rate(0.0), 0.7);
        assert_eq!(r.rate(100.0), 0.7);
        assert_eq!(r.rate(-5.0), 0.7);
    }

    #[test]
    fn cum_rate_is_linear() {
        let r = ConstantRate::new(0.5);
        assert_eq!(r.cum_rate(0.0), 0.0);
        assert_eq!(r.cum_rate(10.0), 5.0);
        assert_eq!(r.cum_rate(-1.0), 0.0);
    }

    #[test]
    fn inverse_cum_rate_roundtrips() {
        let r = ConstantRate::new(0.3);
        for c in [0.5, 1.0, 7.7] {
            let t = r.inverse_cum_rate(c).unwrap();
            assert!((r.cum_rate(t) - c).abs() < 1e-12);
        }
    }

    #[test]
    fn inverse_cum_rate_zero_rate_is_unattainable() {
        let r = ConstantRate::new(0.0);
        assert_eq!(r.inverse_cum_rate(1.0), None);
        // Zero events is always reachable at t=0.
        assert_eq!(r.inverse_cum_rate(0.0), Some(0.0));
    }

    #[test]
    fn max_cum_rate_is_infinite_when_positive() {
        assert_eq!(ConstantRate::new(0.5).max_cum_rate(), f64::INFINITY);
        assert_eq!(ConstantRate::new(0.0).max_cum_rate(), 0.0);
    }
}
