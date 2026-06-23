use enum_dispatch::enum_dispatch;

/// Utility functions for calculating the rate of infection over time
/// See `ScaledRateFn` for how to calculate the inverse cumulative rate for an interval starting
/// at a time other than 0.
///
/// `#[enum_dispatch]` lets the `RateFn` enum (see `storage`) implement this trait by
/// forwarding to its variant without hand-written `match` arms.
#[enum_dispatch]
pub trait InfectiousnessRateFn {
    /// Returns the rate of infection at time `t`
    ///
    /// E.g., Where t=day, `rate(2.0)` -> 1.0 means that at day 2, the person's
    /// rate of infection is 1 person per day.
    fn rate(&self, t: f64) -> f64;

    /// Returns the expected number of infection events that we expect to happen in the interval 0 -> t
    ///
    /// E.g., Where t=day, `cum_rate(4.0)` -> 8.0 means that we would expect to infect 8 people in the
    /// first four days.
    ///
    fn cum_rate(&self, t: f64) -> f64;

    /// Returns the expected time, starting at 0, at which a number of infection `events` will have
    /// occurred.
    ///
    /// E.g., Where t=day, `inverse_cum_rate(6.0)` -> 2.0 means that we would expect
    /// that it would take 2 days to infect 6 people
    fn inverse_cum_rate(&self, events: f64) -> Option<f64>;

    /// The maximum cumulative infectiousness this rate function can ever produce —
    /// `cum_rate(+∞)`, i.e. the total integrated area under `rate`. It is the value at
    /// which [`inverse_cum_rate`](Self::inverse_cum_rate) saturates to `None` (no more
    /// events are possible). A time-unbounded rate (e.g. a constant hazard) returns
    /// `f64::INFINITY`; an empirical curve returns its finite total area.
    ///
    /// Replaces the notion of an explicit infectious duration: the infectious period
    /// ends when integrated infectiousness is exhausted, not at an arbitrary support
    /// boundary.
    fn max_cum_rate(&self) -> f64;
}

/// A utility for scaling and shifting an infectiousness rate function
pub struct ScaledRateFn<'a, T>
where
    T: InfectiousnessRateFn + ?Sized,
{
    pub base: &'a T,
    pub scale: f64,
    pub elapsed: f64,
}

impl<'a, T: ?Sized + InfectiousnessRateFn> ScaledRateFn<'a, T> {
    #[must_use]
    pub fn new(base: &'a T, scale: f64, elapsed: f64) -> Self {
        Self {
            base,
            scale,
            elapsed,
        }
    }
}

impl<T: ?Sized + InfectiousnessRateFn> InfectiousnessRateFn for ScaledRateFn<'_, T> {
    /// Returns the rate of infection at time `t` scaled by a factor of `self.scale`,
    /// and shifted by `self.elapsed`.
    fn rate(&self, t: f64) -> f64 {
        self.base.rate(t + self.elapsed) * self.scale
    }
    /// Returns the cumulative rate for a time interval starting at `self.elapsed`, scaled by a factor
    /// of `self.scale`. For example, say you want to calculate the
    /// interval from 3.0 -> 4.0; you would create a `ScaledRateFn` with an elapsed of 3.0 and
    /// take `cum_rate(1.0)` (the end of the period - the start).
    fn cum_rate(&self, t: f64) -> f64 {
        (self.base.cum_rate(t + self.elapsed) - self.base.cum_rate(self.elapsed)) * self.scale
    }
    /// Returns the expected time, starting at `self.elapsed` by which an expected number of infection
    /// `events` will occur, and sped up by a factor of `self.scale`.
    /// For example, say the current time is 2.1 and you want to calculate the time to infect the
    /// next person (events=1.0). You would create a `ScaledRateFn` with an elapsed of 2.1 and take
    /// `inverse_cum_rate(1.0)`. If you want to increase the rate by a factor of 2.0 (halve the
    /// expected time to infect that person), you would create a `ScaledRateFn` with a scale of 2.0.
    fn inverse_cum_rate(&self, events: f64) -> Option<f64> {
        let elapsed_cum_rate = self.base.cum_rate(self.elapsed);
        Some(
            self.base
                .inverse_cum_rate(events / self.scale + elapsed_cum_rate)?
                - self.elapsed,
        )
    }
    /// The remaining cumulative infectiousness from `self.elapsed` onward, scaled by
    /// `self.scale`: `(base.max_cum_rate() − base.cum_rate(elapsed)) · scale`. Propagates
    /// `f64::INFINITY` from an unbounded base.
    fn max_cum_rate(&self) -> f64 {
        (self.base.max_cum_rate() - self.base.cum_rate(self.elapsed)) * self.scale
    }
}

#[cfg(test)]
mod test {
    use ixa::assert_almost_eq;

    use super::{InfectiousnessRateFn, ScaledRateFn};
    use crate::rate::EmpiricalRate;

    // A flat empirical curve of height 2.0 on [0, 5] (rate 0 outside that range) — a
    // finite-support stand-in for a "constant 2.0 for 5 days" profile. Unlike the
    // reference's capped `ConstantRate`, this repo's `ConstantRate` is unbounded in
    // time, so the elapsed/exhaustion behavior is exercised here via `EmpiricalRate`.
    fn flat_capped() -> EmpiricalRate {
        EmpiricalRate::new(vec![[0.0, 2.0], [5.0, 2.0]], 1.0)
    }

    #[test]
    fn test_scale_rate_fn() {
        let rate_fn = flat_capped();
        let scaled_rate_fn = ScaledRateFn {
            base: &rate_fn,
            scale: 2.0,
            elapsed: 0.0,
        };
        assert_almost_eq!(scaled_rate_fn.rate(0.0), 4.0, 0.0);
        assert_almost_eq!(scaled_rate_fn.rate(5.0), 4.0, 0.0);
    }
    #[test]
    fn test_scale_rate_fn_with_elapsed() {
        let rate_fn = flat_capped();
        let scaled_rate_fn = ScaledRateFn {
            base: &rate_fn,
            scale: 2.0,
            elapsed: 3.0,
        };
        assert_almost_eq!(scaled_rate_fn.rate(0.0), 4.0, 0.0);
        // Since the elapsed is 3.0, the rate at t=4.0 is past the total infectious
        // period (3.0 + 4.0 = 7.0), so the rate is 0.0.
        assert_almost_eq!(scaled_rate_fn.rate(4.0), 0.0, 0.0);
    }
    #[test]
    fn test_scale_rate_fn_cum_rate() {
        let rate_fn = flat_capped();
        let scaled_rate_fn = ScaledRateFn {
            base: &rate_fn,
            scale: 2.0,
            elapsed: 3.0,
        };
        assert_almost_eq!(scaled_rate_fn.cum_rate(1.0), 4.0, 0.0);
        assert_almost_eq!(scaled_rate_fn.cum_rate(2.0), 8.0, 0.0);
        // The cumulative rate for t=3.0 with an elapsed t=3.0 is still
        // only 2 days, since the infectiousness period ends at 5.0
        assert_almost_eq!(scaled_rate_fn.cum_rate(3.0), 8.0, 0.0);
    }
    #[test]
    fn test_scale_rate_fn_inverse_cum_rate() {
        let rate_fn = flat_capped();
        let scaled_rate_fn = ScaledRateFn {
            base: &rate_fn,
            scale: 2.0,
            elapsed: 3.0,
        };
        assert_eq!(scaled_rate_fn.inverse_cum_rate(4.0), Some(1.0));
        assert_eq!(scaled_rate_fn.inverse_cum_rate(8.0), Some(2.0));
        assert_eq!(scaled_rate_fn.inverse_cum_rate(11.0), None);
    }
    #[test]
    fn test_scale_rate_fn_max_cum_rate() {
        let rate_fn = flat_capped();
        // Full curve area = 2.0 · 5.0 = 10.0.
        assert_almost_eq!(rate_fn.max_cum_rate(), 10.0, 0.0);
        // From elapsed 3.0 onward, the remaining area is 2.0 · 2.0 = 4.0, scaled by 2.0.
        let scaled_rate_fn = ScaledRateFn {
            base: &rate_fn,
            scale: 2.0,
            elapsed: 3.0,
        };
        assert_almost_eq!(scaled_rate_fn.max_cum_rate(), 8.0, 0.0);
    }
}
