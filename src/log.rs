use std::io::{self, IsTerminal, Write};

use env_logger::fmt::Formatter;
use log::Record;

pub fn init() {
    let mut builder = env_logger::builder();

    if under_systemd() {
        builder.format(systemd_log_format);
    }

    builder
        .format_timestamp_millis()
        .filter_level(default_log_level())
        .parse_default_env()
        .init();
}

fn default_log_level() -> log::LevelFilter {
    if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    }
}

fn systemd_log_format(buf: &mut Formatter, record: &Record) -> io::Result<()> {
    writeln!(
        buf,
        "<{}>{}: {}",
        match record.level() {
            log::Level::Error => 3,
            log::Level::Warn => 4,
            log::Level::Info => 6,
            log::Level::Debug => 7,
            log::Level::Trace => 7,
        },
        record.target(),
        record.args()
    )
}

fn under_systemd() -> bool {
    std::env::var("SYSTEMD_EXEC_PID").is_ok() && !std::io::stdout().is_terminal()
}
