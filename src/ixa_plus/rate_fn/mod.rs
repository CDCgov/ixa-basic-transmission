mod generator;
mod rate_fn_trait;

pub use crate::define_rate;
pub use generator::*;
pub use rate_fn_trait::*;

// Rate Functions
macro_rules! rate_fn_enum {
    ($($name:ident),*) => {
        pub enum RateFn {
            $($name($name)),+
        }
        impl InfectiousnessRateFn for RateFn {
            fn rate(&self, t: f64) -> f64 {
                match self {
                    $(Self::$name(rate) => rate.rate(t),)+
                }
            }
            fn cum_rate(&self, t: f64) -> f64 {
                match self {
                    $(Self::$name(rate) => rate.cum_rate(t),)+
                }
            }
            fn inverse_cum_rate(&self, events: f64) -> Option<f64> {
                match self {
                    $(Self::$name(rate) => rate.inverse_cum_rate(events),)+
                }
            }
            fn infection_duration(&self) -> f64 {
                match self {
                    $(Self::$name(rate) => rate.infection_duration(),)+
                }
            }
        }
    };
}

mod constant_rate;
pub use constant_rate::*;
rate_fn_enum!(ConstantRate);
