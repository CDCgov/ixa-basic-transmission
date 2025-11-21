use crate::simulation_event::SimulationEvent;
use crate::{ext::*, infection_status::*, output_manager::OutputManagerExt};
use ixa::prelude::*;

define_rng!(ContactRng);
define_rng!(TransmissionRng);

pub trait TransmissionManagerExt: PluginContext + OutputManagerExt {
    fn get_next_contact(&self, person_id: PersonId) -> Option<PersonId> {
        let mut contact_id = None;
        // Ensure we don't return the same id
        let total_people = self.get_current_population();
        if total_people > 1 {
            loop {
                contact_id = self.sample_person(ContactRng, ());
                if contact_id != Some(person_id) {
                    break;
                }
            }
        }
        contact_id
    }
    // Infection attempt function for a context and given `PersonId`
    fn attempt_transmission(&mut self, infector: PersonId) -> Option<PersonId> {
        // Get a contact
        let next_contact = self.get_next_contact(infector)?;

        self.emit_event(SimulationEvent::Contact {
            t: self.get_current_time(),
            person_id: infector,
            contact_id: next_contact,
        });

        // if the person is not susceptible, fail the attempt.
        if !self
            .get_person_property(next_contact, InfectionStatus)
            .is_susceptible()
        {
            return None;
        }

        // Reject based on relative transmission modifiers
        if !self.sample_bool(
            TransmissionRng,
            self.get_relative_total_transmission(infector, next_contact),
        ) {
            // If the rejection sample fails, return None
            return None;
        }

        // Infection succeeds
        self.infect_person(next_contact, Some(infector), Some(self.get_current_time()));
        // Return the ID of the newly infected person
        Some(next_contact)
    }

    // Apply any modifiers that impact transmission
    fn get_relative_total_transmission(&self, _infector: PersonId, _infectee: PersonId) -> f64 {
        1.0
    }
}

impl<C> TransmissionManagerExt for C where C: PluginContext + OutputManagerExt {}
