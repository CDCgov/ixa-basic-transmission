use ixa::prelude::*;

/// Calculate the maximum possible scaling factor for total infectiousness
/// for a person, given information we know at the time of a forecast.
/// The modifier used for intrinsic infectiousness is ignored because all modifiers must
/// be less than or equal to one.
pub fn forecasted_maximum(_context: &impl PluginContext, _person_id: PersonId) -> f64 {
    1.0
}

/// Calculate the scaling factor that accounts for the total infectiousness
/// for a person, given factors related to their environment, such as the number of people
/// they come in contact with or how close they are.
/// This is used to scale the intrinsic infectiousness function of that person.
/// All modifiers of the infector's intrinsic infectiousness are aggregated and returned
/// as a single float to multiply by the base total infectiousness.
/// This assumes that transmission modifiers of total infectiousness are independent of
/// the setting type and are linear
pub fn actual(_context: &impl PluginContext, _person_id: PersonId) -> f64 {
    1.0
}
