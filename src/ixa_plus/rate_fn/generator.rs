use super::InfectiousnessRateFn;
use super::RateFn;
use crate::ixa_plus::type_index::{TypeIndex, TypeIndexCategory, TypeIndexMap};
use ixa::HashMap;
use ixa::prelude::*;

impl TypeIndexCategory for RateFn {}

pub trait RateFnGenerator<C: PluginContext>: Clone {
    fn name(&self) -> &'static str;
    fn assign(&self, context: &C, person_id: PersonId) -> RateFn;
}
pub struct RateFnDataContainer {
    // TODO: When we have access to PersonId index, we can make this a PersonVec
    per_person_rates: TypeIndexMap<RateFn, HashMap<PersonId, usize>>,
    rate_instances: Vec<RateFn>,
}

impl Default for RateFnDataContainer {
    fn default() -> Self {
        Self {
            per_person_rates: TypeIndexMap::new(),
            rate_instances: Vec::new(),
        }
    }
}

impl RateFnDataContainer {
    pub fn add_instance(&mut self, instance: RateFn) -> usize {
        self.rate_instances.push(instance);
        self.rate_instances.len() - 1
    }
    pub fn assign_rate_fn<C: PluginContext, G: RateFnGenerator<C> + TypeIndex<RateFn>>(
        &mut self,
        person_id: PersonId,
        instance: RateFn,
    ) {
        let index = self.add_instance(instance);
        let person_vec = self
            .per_person_rates
            .get_mut_or_insert::<G>(HashMap::default());
        person_vec.insert(person_id, index);
    }
    pub fn get_rate_fn<T: TypeIndex<RateFn>>(&self, person_id: PersonId) -> Option<&RateFn> {
        let index = self.per_person_rates.get::<T>()?.get(&person_id)?;
        self.rate_instances.get(*index)
    }
}

define_data_plugin!(RateFnPlugin, RateFnDataContainer, |_context| {
    RateFnDataContainer::default()
});

pub trait RateFnExt: PluginContext {
    fn assign_rate<G: RateFnGenerator<Self> + TypeIndex<RateFn>>(
        &mut self,
        person_id: PersonId,
        rate: G,
    ) {
        let instance = rate.assign(self, person_id);
        let data = self.get_data_mut(RateFnPlugin);
        data.assign_rate_fn::<Self, G>(person_id, instance);
    }
    fn get_person_rate_fn<G: RateFnGenerator<Self> + TypeIndex<RateFn>>(
        &self,
        person_id: PersonId,
        _generator: G,
    ) -> &impl InfectiousnessRateFn {
        let data = self.get_data(RateFnPlugin);
        let rate_fn = data
            .get_rate_fn::<G>(person_id)
            .expect("Rate function not found");
        rate_fn
    }
}

impl<T> RateFnExt for T where T: PluginContext {}

#[macro_export]
macro_rules! define_rate {
    (
        $name:ident,
        |$ctx:ident, $person_id:ident| $body:block
    ) => {
        #[derive(Clone)]
        pub struct $name;

        $crate::type_index!($crate::ixa_plus::rate_fn::RateFn, $name);

        impl<C: PluginContext> crate::ixa_plus::rate_fn::RateFnGenerator<C> for $name {
            fn name(&self) -> &'static str {
                stringify!($name)
            }
            fn assign(
                &self,
                $ctx: &C,
                $person_id: ixa::people::PersonId,
            ) -> crate::ixa_plus::rate_fn::RateFn {
                $body
            }
        }
    };
}
