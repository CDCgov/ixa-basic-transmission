use crate::rate::InvertibleRate;
use ixa::prelude::*;
use rand_distr::Exp;

define_rng!(TransmissionRng);

/// Infection loop transmission dynamics.
///
/// Transmission occurs due to a combination of three factors: (potential)
/// infector's infectiousness, the contact rate between the infector and
/// (potential) infectees, and the susceptibility of (potential) infectees.
///
/// A transmission "attempt" is when a transmission would occur if the
/// potential infectee is susceptible. Attempts account for infectiousness
/// and contact rates.
///
/// The loop algorithm is:
/// 1. Draw attempt proposal times based on majorizing transmission attempt rates.
/// 2. Thin proposed attempts to attempts.
/// 3. Attempts become transmissions based on the potential infectee's susceptibility.
pub trait Transmission<E>: ixa::ContextBase + ixa::ContextRandomExt
where
    E: Entity,
{
    /// Majorizing rate for transmission attempts from `infector`, indexed by
    /// time from now (`t = 0` is the current simulation time).
    fn majorizing_transmission_attempt_rate_fn(&self, infector: EntityId<E>)
        -> impl InvertibleRate;

    /// Transmission attempt rate, bounded at all times by the majorizing attempt rate.
    /// `None` signals that the rate is zero and will continue to be zero,
    /// i.e., that the infection loop for this infector should break.
    fn transmission_attempt_rate(&self, person: EntityId<E>) -> Option<f64>;

    /// Given a transmission attempt from `infector`, who is the candidate infectee?
    /// `None` breaks the loop.
    // TO DO: `None` is probably a sign of inconsistency in the selection of the attempt rate!
    fn select_infectee(&self, infector: EntityId<E>) -> Option<EntityId<E>>;

    /// Probability that a person, subjected to an infection attempt, will be infected.
    fn susceptibility(&self, person: EntityId<E>) -> f64;

    /// Do the transmission. This should, at a minimum, trigger the infectee's
    /// own infection loop,
    fn transmit(&mut self, infector: EntityId<E>, infectee: EntityId<E>);

    /// Schedule the next attempt proposal.
    ///
    /// This function begins the transmission loop for an infector.
    fn schedule_next_transmission_attempt_proposal(&mut self, infector: EntityId<E>)
    where
        Context: Transmission<E>,
    {
        // Determine if & when the next attempt proposal will occur. The majorizing
        // rate function is indexed by time from now, so inverting an Exp(1) draw
        // gives the delay to the proposal. If `e` is beyond the support of the
        // inverse cumulative there is no next event and the loop ends.
        let e = self.sample_distr(TransmissionRng, Exp::new(1.0).unwrap());
        let proposal = {
            let f = self.majorizing_transmission_attempt_rate_fn(infector);
            f.inverse_cum_rate(e).map(|delay| (delay, f.rate(delay)))
        };

        if let Some((delay, majorizing_rate)) = proposal {
            let t = self.get_current_time() + delay;
            self.add_plan(t, move |ctx| {
                ctx.handle_transmission_attempt_proposal(infector, majorizing_rate);
            });
        };
    }

    /// Handle a transmission attempt proposal, thinning attempt proposals to attempts
    /// based on the proposal rate and attempt rates, then thinning
    /// attempts to transmissions based on susceptibility.
    fn handle_transmission_attempt_proposal(&mut self, infector: EntityId<E>, majorizing_rate: f64)
    where
        Context: Transmission<E>,
    {
        // If the attempt rate is None, break the infection loop
        if let Some(rate) = self.transmission_attempt_rate(infector) {
            // The scheduler evaluated the bound at `delay + (now - t_inf)` and the
            // handler evaluates the rate at `(now + delay) - t_inf`: the same
            // instant computed two ways, so allow floating-point drift.
            assert!(
                rate <= majorizing_rate + 1e-10,
                "person {infector:?}: attempt rate {rate} exceeds majorizing attempt proposal rate {majorizing_rate}"
            );

            // Thin the attempt proposals based on majorizing and target rates. A
            // tight bound (the common case) accepts without drawing; a zero bound
            // (a proposal exactly at the end of the support) can never attempt.
            let attempt = majorizing_rate > 0.0
                && (rate >= majorizing_rate
                    || self.sample_bool(TransmissionRng, rate / majorizing_rate));
            if attempt {
                // Thin the attempts to infections based on susceptibility, drawing
                // only when the outcome is uncertain.
                if let Some(infectee) = self.select_infectee(infector) {
                    let p = self.susceptibility(infectee);
                    let infected = p >= 1.0 || (p > 0.0 && self.sample_bool(TransmissionRng, p));
                    if infected {
                        self.transmit(infector, infectee);
                    } // else: infectee non-susceptibility prevented transmission
                } // else: no person to sample
            } // else: contact+infectiousness were not sufficient for transmission

            self.schedule_next_transmission_attempt_proposal(infector);
        } // else: one of the contact rate or infectiousness was None; stop the loop for this infector
    }
}
