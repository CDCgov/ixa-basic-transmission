use ixa::{IxaEvent, PersonId};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "type")]
pub enum SimulationEvent {
    Infection {
        t: f64,
        person_id: PersonId,
    },
    Contact {
        t: f64,
        person_id: PersonId,
        contact_id: PersonId,
    },
    ForecastRejected {
        t: f64,
        person_id: PersonId,
    },
}
impl IxaEvent for SimulationEvent {}
