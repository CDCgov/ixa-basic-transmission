use std::sync::OnceLock;

static LOGGER_INITIALIZED: OnceLock<bool> = OnceLock::new();
static LOG_LEVEL: OnceLock<log::LevelFilter> = OnceLock::new();

/// Re-export LevelFilter for public use
pub use log::LevelFilter;

/// Initialize the logger with a custom formatter
pub fn init() {
    if LOGGER_INITIALIZED.get().copied().unwrap_or(false) {
        return;
    }

    let level = LOG_LEVEL.get().copied();

    let mut builder = env_logger::Builder::from_default_env();

    // Override with our level if set, otherwise use env var or default
    if let Some(level) = level {
        builder.filter_level(level);
    }

    builder
        .format(|buf, record| {
            use std::io::Write;

            // ANSI color codes for log levels
            let (color, level_str) = match record.level() {
                log::Level::Error => ("\x1b[31m", "ERROR"), // Red
                log::Level::Warn => ("\x1b[33m", "WARN"),   // Yellow
                log::Level::Info => ("\x1b[32m", "INFO"),   // Green
                log::Level::Debug => ("\x1b[36m", "DEBUG"), // Cyan
                log::Level::Trace => ("\x1b[35m", "TRACE"), // Magenta
            };
            let reset = "\x1b[0m";

            // Check if this log is from our crate
            let crate_name = record
                .module_path()
                .and_then(|path| path.split("::").next());

            let is_our_crate = crate_name
                .map(|name| name == "ixa_basic_transmission")
                .unwrap_or(false);

            // Format the log line
            if is_our_crate {
                // Internal logs: no crate name
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
                // External logs: include crate name
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

/// Set the log level filter
pub fn set_log_level(level: LevelFilter) {
    LOG_LEVEL.set(level).unwrap_or_else(|_| {
        // Logger already initialized, that's okay
    });
    init();
}

/// Initialize logger with default level (Info) if not already initialized
pub fn init_default() {
    if LOG_LEVEL.get().is_none() {
        LOG_LEVEL
            .set(LevelFilter::Info)
            .unwrap_or_else(|_| unreachable!());
    }
    init();
}

/// Re-export log macros for convenience
pub use log::{debug, error, info, trace, warn};
