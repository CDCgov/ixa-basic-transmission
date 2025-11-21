use crate::format_iter;
use crate::ixa_plus::log;
use anyhow::Result;
use ixa::prelude::*;
use rand_distr::weighted::WeightedIndex;
use std::fmt::Display;

define_rng!(PopulationRng);

type AssignFn<C> = fn(&mut C, PersonId) -> Result<()>;

pub trait PopulationManagerExt: PluginContext {
    fn init_population<T: Display>(
        &mut self,
        population_size: usize,
        proportions: Vec<(f64, (T, AssignFn<Self>))>,
    ) -> Result<usize> {
        let (mut weights, assign_fns): (Vec<_>, Vec<_>) = proportions.into_iter().unzip();

        // Validate total weight
        let total_weight: f64 = weights.iter().sum();
        if total_weight > 1.0 {
            anyhow::bail!("Proportions must sum to 1.0 or less (got {total_weight:.3})");
        }

        let leftover = 1.0 - total_weight;
        if leftover > 0.0 {
            weights.push(leftover)
        }

        let dist = WeightedIndex::new(&weights)?;

        let mut counts = assign_fns
            .iter()
            .map(|(label, _)| (label, 0usize))
            .collect::<Vec<_>>();
        for _ in 0..population_size {
            let person_id = self.add_person(())?;
            let index = self.sample_distr(PopulationRng, &dist);
            if let Some((_, apply)) = assign_fns.get(index) {
                apply(self, person_id)?;
                counts[index].1 += 1;
            }
        }
        log::info!(
            "Seeded population of size {} with\n{}",
            population_size,
            format_iter!(counts, |(label, count)| "{label}: {count}")
        );
        Ok(population_size)
    }
}

impl<C> PopulationManagerExt for C where C: PluginContext {}
