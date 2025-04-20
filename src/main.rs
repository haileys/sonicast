use std::env::VarError;
use std::fmt::Display;
use std::str::FromStr;

use anyhow::Result;

mod app;
mod log;
mod mpd;
mod subsonic;
mod util;

#[tokio::main]
async fn main() -> Result<()> {
    log::init();

    let config = config();
    app::run(&config).await
}

fn config() -> app::Config {
    app::Config {
        listen: env("SONICAST_LISTEN"),
        subsonic: subsonic(),
        mpd: mpd(),
    }
}

fn subsonic() -> subsonic::Config {
    subsonic::Config {
        base_url: env("SUBSONIC_URL"),
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
