use std::env::VarError;
use std::fmt::Display;
use std::str::FromStr;

use anyhow::Result;

// use ffmpeg_next::format;

mod app;
mod mpd;
mod subsonic;
mod util;

#[tokio::main]
async fn main() -> Result<()> {
    init_log();

    let config = app::Config {
        subsonic: subsonic(),
        mpd: mpd(),
    };

    app::run(&config).await?;

    Ok(())
}

fn init_log() {
    env_logger::builder()
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

fn subsonic() -> subsonic::Config {
    subsonic::Config {
        base_url: env("SUBSONIC_URL"),
        username: env("SUBSONIC_USERNAME"),
        password: env("SUBSONIC_PASSWORD"),
    }
}

fn mpd() -> mpd::Config {
    mpd::Config {
        socket: env("MPD_SOCKET"),
    }
}

fn env<T: FromStr<Err: Display>>(name: &str) -> T {
    let value = match std::env::var(name) {
        Ok(value) => value,
        Err(VarError::NotPresent) => panic!("missing env var: {name}"),
        Err(VarError::NotUnicode(_)) => panic!("env var is invalid utf-8: {name}"),
    };

    match value.parse() {
        Ok(value) => value,
        Err(err) => panic!("invalid format for env var: {name}: {err}"),
    }
}
