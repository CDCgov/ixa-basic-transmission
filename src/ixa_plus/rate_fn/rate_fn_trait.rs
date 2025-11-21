/// Utility functions for calculating the rate of infection over time
/// See `ScaledRateFn` for how to calculate the inverse cumulative rate for an interval starting
/// at a time other than 0.
///
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

    fn infection_duration(&self) -> f64;
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

impl<'a, T> ScaledRateFn<'a, T>
where
    T: ?Sized + InfectiousnessRateFn,
{
    #[must_use]
    pub fn new(base: &'a T, scale: f64, elapsed: f64) -> Self {
        Self {
            base,
            scale,
            elapsed,
        }
    }
}

impl<T> InfectiousnessRateFn for ScaledRateFn<'_, T>
where
    T: ?Sized + InfectiousnessRateFn,
{
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
    /// Returns the duration of infectiousness.
    fn infection_duration(&self) -> f64 {
        self.base.infection_duration() - self.elapsed
    }
}
