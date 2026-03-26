use ixa_basic_transmission::ixa_plus::params_macro::IxaParameters;
use ixa_basic_transmission::ext::OutputManagerExt;

fn main() {
    #[cfg(debug_assertions)]
    ixa_basic_transmission::ixa_plus::log::set_log_level(ixa_basic_transmission::ixa_plus::log::LevelFilter::Debug);

    #[cfg(not(debug_assertions))]
    ixa_basic_transmission::ixa_plus::log::init_default();

    let params = ixa_basic_transmission::params::Params::from_args();
    let mut context = ixa_basic_transmission::model::setup(params).unwrap();
    context.execute();
    context.log_stats();
}