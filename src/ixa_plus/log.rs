pub use log::LevelFilter;
pub use log::{debug, error, info, trace, warn};

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::OnceLock;

    static LOGGER_INITIALIZED: OnceLock<bool> = OnceLock::new();
    static LOG_LEVEL: OnceLock<log::LevelFilter> = OnceLock::new();

    pub fn init() {
        if LOGGER_INITIALIZED.get().copied().unwrap_or(false) {
            return;
        }

        let level = LOG_LEVEL.get().copied();

        let mut builder = env_logger::Builder::from_default_env();

        if let Some(level) = level {
            builder.filter_level(level);
        }

        builder
            .format(|buf, record| {
                use std::io::Write;

                let (color, level_str) = match record.level() {
                    log::Level::Error => ("\x1b[31m", "ERROR"),
                    log::Level::Warn => ("\x1b[33m", "WARN"),
                    log::Level::Info => ("\x1b[32m", "INFO"),
                    log::Level::Debug => ("\x1b[36m", "DEBUG"),
                    log::Level::Trace => ("\x1b[35m", "TRACE"),
                };
                let reset = "\x1b[0m";

                let crate_name = record
                    .module_path()
                    .and_then(|path| path.split("::").next());

                let is_our_crate = crate_name
                    .map(|name| name == "ixa_basic_transmission")
                    .unwrap_or(false);

                if is_our_crate {
                    writeln!(
                        buf,
                        "{}{}{} {}{}",
                        color,
                        level_str,
                        reset,
                        record.args(),
                        reset
                    )
                } else {
                    let crate_display = crate_name.unwrap_or("unknown");
                    writeln!(
                        buf,
                        "{}{}{} [{}] {}{}",
                        color,
                        level_str,
                        reset,
                        crate_display,
                        record.args(),
                        reset
                    )
                }
            })
            .init();

        LOGGER_INITIALIZED
            .set(true)
            .unwrap_or_else(|_| unreachable!());
    }

    pub fn set_log_level(level: log::LevelFilter) {
        LOG_LEVEL.set(level).unwrap_or(());
        init();
    }

    pub fn init_default() {
        if LOG_LEVEL.get().is_none() {
            LOG_LEVEL
                .set(log::LevelFilter::Info)
                .unwrap_or_else(|_| unreachable!());
        }
        init();
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{init, init_default, set_log_level};
