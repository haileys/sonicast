use std::convert::Infallible;
use std::str::FromStr;

use anyhow::{bail, Result};
use derive_more::FromStr;
use serde::{Serialize, Deserialize};

use crate::mpd::protocol::Attributes;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Id(String);

impl Id {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Id {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Infallible> {
        Ok(Id(s.to_string()))
    }
}

#[derive(Debug)]
pub struct Playlist {
    pub items: Vec<PlaylistItem>,
}

#[derive(Debug, Clone)]
pub struct PlaylistItem {
    pub file: String,
    #[allow(unused)]
    pub pos: i64,
    pub id: Id,
    #[allow(unused)]
    pub name: Option<String>,
    #[allow(unused)]
    pub title: Option<String>,
}

#[derive(Debug)]
pub struct Changed {
    subsystems: Vec<String>,
}

impl Changed {
    pub fn from_attributes(attrs: &Attributes) -> Result<Self> {
        let subsystems = attrs.get_all("changed")
            .map(|v| v.to_string())
            .collect();

        Ok(Changed { subsystems })
    }

    pub fn events(&self) -> impl Iterator<Item = MpdEvent> + '_ {
        self.subsystems.iter()
            .filter_map(|subsystem| {
                match subsystem.parse() {
                    Ok(event) => Some(event),
                    Err(()) => {
                        log::warn!("unknown subsystem: {subsystem}");
                        None
                    }
                }
            })
    }
}

#[derive(Debug)]
pub enum MpdEvent {
    Playlist,
    Player,
    Options,
    Mixer,
}

impl FromStr for MpdEvent {
    type Err = ();

    fn from_str(s: &str) -> Result<MpdEvent, ()> {
        match s {
            "player" => Ok(MpdEvent::Player),
            "playlist" => Ok(MpdEvent::Playlist),
            "options" => Ok(MpdEvent::Options),
            "mixer" => Ok(MpdEvent::Mixer),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PlaybackState {
    Stop,
    Pause,
    Play,
}

#[derive(Debug, Copy, Clone, FromStr)]
pub struct Seconds(pub f64);

#[derive(Debug)]
pub struct Status {
    pub state: PlaybackState,
    pub song: Option<usize>,
    pub song_id: Option<Id>,
    pub elapsed: Option<Seconds>,
    pub duration: Option<Seconds>,
    #[allow(unused)]
    pub audio_format: Option<String>,
    pub playlist_version: u32,
    pub repeat: bool,
    pub random: bool,
    pub single: bool,
    pub volume: Option<usize>,
}

impl Status {
    pub fn from_attributes(attrs: &Attributes) -> Result<Self> {
        let state = match attrs.get_one("state") {
            Some("play") => PlaybackState::Play,
            Some("pause") => PlaybackState::Pause,
            Some("stop") => PlaybackState::Stop,
            Some(state) => bail!("unknown player state: {state}"),
            None => bail!("missing player state"),
        };

        Ok(Status {
            state,
            song: attrs.get_opt("song")?,
            song_id: attrs.get_opt("songid")?,
            elapsed: attrs.get_opt("elapsed")?,
            duration: attrs.get_opt("duration")?,
            audio_format: attrs.get_opt("audio")?,
            playlist_version: attrs.get("playlist")?,
            repeat: attrs.get_bool("repeat")?,
            random: attrs.get_bool("random")?,
            single: attrs.get_bool("single")?,
            volume: attrs.get_opt("volume")?,
        })
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum ReplayGainMode {
    None,
    Track,
    Album,
    Auto,
}

impl FromStr for ReplayGainMode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "none" => Ok(ReplayGainMode::None),
            "track" => Ok(ReplayGainMode::Track),
            "album" => Ok(ReplayGainMode::Album),
            "auto" => Ok(ReplayGainMode::Auto),
            _ => anyhow::bail!("unknown replay_gain_mode: {s}")
        }
    }
}
