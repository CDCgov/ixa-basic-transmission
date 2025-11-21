use crate::ext::ParametersExt;
use crate::infection_status::*;
use crate::simulation_event::SimulationEvent;
use anyhow::Result;
use ixa::{PersonPropertyChangeEvent, prelude::*};
use std::io::BufWriter;
use std::io::Write;
use std::path::PathBuf;

const OUTPUT_DIR: &str = "output";

fn create_output_file(filename: &str) -> Result<std::fs::File> {
    std::fs::create_dir_all(OUTPUT_DIR)
        .and_then(|_| {
            let mut path = PathBuf::from(OUTPUT_DIR);
            path.push(filename);
            std::fs::File::create(path)
        })
        .map_err(|e| anyhow::anyhow!("Failed to create output file: {}", e))
}

struct Counts {
    total_infections: usize,
    forecasts_rejected: usize,
    daily_incidence: Vec<usize>,
}
impl Counts {
    fn new(sim_length: f64) -> Self {
        Self {
            total_infections: 0,
            forecasts_rejected: 0,
            daily_incidence: Vec::with_capacity(sim_length.floor() as usize),
        }
    }
    fn add_forecast_rejection(&mut self) {
        self.forecasts_rejected += 1;
    }
    fn add_infection(&mut self, status: Status) {
        self.total_infections += 1;
        if let Some(infection_time) = status.infection_time() {
            let day_index = infection_time.floor() as usize;
            if day_index >= self.daily_incidence.len() {
                self.daily_incidence.resize(day_index + 1, 0);
            }
            self.daily_incidence[day_index] += 1;
        }
    }
}

// Store writers in a plugin data container
struct OutputDataContainer {
    counts: Counts,
    json_writer: BufWriter<std::fs::File>,
    daily_incidence_writer: ixa::csv::Writer<std::fs::File>,
}

impl OutputDataContainer {
    fn write_daily_incidence(&mut self) {
        self.daily_incidence_writer
            .write_record(&["t", "incidence"])
            .expect("Failed to write header");
        for (day, incidence) in self.counts.daily_incidence.iter().enumerate() {
            self.daily_incidence_writer
                .write_record(&[day.to_string(), incidence.to_string()])
                .expect("Failed to write daily incidence");
        }
    }
}

define_data_plugin!(OutputPlugin, OutputDataContainer, |context| {
    let events_file = create_output_file("events.jsonl").unwrap();
    let events_writer = BufWriter::new(events_file);

    let csv_writer =
        ixa::csv::Writer::from_path(PathBuf::from(OUTPUT_DIR).join("daily_incidence.csv"))
            .expect("Failed to create incidence writer");

    let max_time = context.param_max_time();
    OutputDataContainer {
        counts: Counts::new(*max_time),
        json_writer: events_writer,
        daily_incidence_writer: csv_writer,
    }
});

pub trait OutputManagerExt: PluginContext {
    fn capture_output(&mut self) {
        // Send infection events
        self.subscribe_to_event(
            |context, event: PersonPropertyChangeEvent<InfectionStatus>| {
                if !event.current.is_infectious() {
                    return;
                }
                let data = context.get_data_mut(OutputPlugin);

                if event.current.is_incidence() {
                    data.counts.add_infection(event.current);

                    let output = SimulationEvent::Infection {
                        t: event.current.infection_time().unwrap(),
                        person_id: event.person_id,
                    };
                    context.write_event(output).expect("Failed to write event");
                }
            },
        );

        // Send forecast rejected events
        self.subscribe_to_event(move |context, event: SimulationEvent| {
            if let SimulationEvent::ForecastRejected { .. } = event {
                let data = context.get_data_mut(OutputPlugin);
                data.counts.add_forecast_rejection();
            }
            context.write_event(event).expect("Failed to write event")
        });
    }

    fn log_stats(&mut self) {
        self.get_data_mut(OutputPlugin).write_daily_incidence();
        let data = self.get_data(OutputPlugin);

        log::info!(
            "Expected mean infectious period: {:.3}",
            self.param_infection_duration().mean()
        );
        log::info!(
            "Expected mean infection rate: {:.3}",
            self.param_infection_rate().mean(),
        );
        log::info!("Total infections: {}", data.counts.total_infections);
        let attack_rate =
            data.counts.total_infections as f64 / self.get_current_population() as f64;
        log::info!("Attack rate: {:.3}", attack_rate);
        let total_infections = data.counts.total_infections as f64;
        let rejected_forecasts = data.counts.forecasts_rejected as f64;
        let forecast_efficiency = if total_infections > 0.0 {
            1.0 - (rejected_forecasts / (total_infections + rejected_forecasts))
        } else {
            0.0
        };
        log::info!("Forecast efficiency: {:.3}", forecast_efficiency);
    }

    fn write_event(&mut self, event: SimulationEvent) -> Result<()> {
        let plugin_data = self.get_data_mut(OutputPlugin);
        serde_json::to_writer(&mut plugin_data.json_writer, &event)?;
        writeln!(&mut plugin_data.json_writer)?;
        Ok(())
    }
}

impl<C> OutputManagerExt for C where C: PluginContext {}
