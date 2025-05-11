use std::env::VarError;
use std::fmt::Display;
use std::str::FromStr;

use anyhow::Result;

mod logging;
mod mpd;
mod player;
mod podcasts;
mod subsonic;
mod util;

#[tokio::main]
async fn main() -> Result<()> {
    logging::init();

    let config = config();
    player::run(&config).await
}

fn config() -> player::Config {
    player::Config {
        listen: env("SONICAST_LISTEN"),
        subsonic_url: env("SUBSONIC_URL"),
        mpd: mpd(),
        podcasts: podcasts(),
    }
}

fn podcasts() -> Option<podcasts::Config> {
    let server_url = opt_env("PODCASTS_URL")?;

    Some(podcasts::Config {
        server_url,
        episode_prefix: env("PODCAST_EPISODE_PREFIX"),
    })
}

fn mpd() -> mpd::Config {
    mpd::Config {
        socket: env("MPD_SOCKET"),
    }
}

fn env<T: FromStr<Err: Display>>(name: &str) -> T {
    match opt_env(name) {
        Some(value) => value,
        None => { panic!("missing env var: {name}") }
    }
}

fn opt_env<T: FromStr<Err: Display>>(name: &str) -> Option<T> {
    let value = match std::env::var(name) {
        Ok(value) => value,
        Err(VarError::NotPresent) => { return None }
        Err(VarError::NotUnicode(_)) => panic!("env var is invalid utf-8: {name}"),
    };

    match value.parse() {
        Ok(value) => Some(value),
        Err(err) => panic!("invalid format for env var: {name}: {err}"),
    }
}
