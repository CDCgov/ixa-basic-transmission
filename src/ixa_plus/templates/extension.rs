use anyhow::Result;
use ixa::prelude::*;

define_rng!(EXT_NAMERng);

pub trait EXT_NAMEExt: PluginContext {
    fn example(&mut self) -> Result<()> {
        log::info!("An example extension method");
        Ok(())
    }
}

impl<C> EXT_NAMEExt for C where C: PluginContext {}
