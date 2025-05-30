use derive_more::From;
use serde::{Deserialize, Serialize};

use crate::{podcasts::PodcastEpisode, subsonic::types::{RadioId, RadioStation, Track, TrackDetails, TrackId}};

#[derive(Debug, Serialize, Deserialize)]
pub struct AirsonicTrack {
    pub id: AirsonicTrackId,
    #[serde(flatten)]
    pub details: TrackDetails,
}

impl From<Track> for AirsonicTrack {
    fn from(track: Track) -> Self {
        AirsonicTrack {
            id: track.id.into(),
            details: track.details,
        }
    }
}

// airsonic treats radio stations as tracks in its own data model
// see airsonic-refix/api.tst:normalizeRadioStation
impl From<RadioStation> for AirsonicTrack {
    fn from(station: RadioStation) -> Self {
        AirsonicTrack {
            id: station.id.into(),
            details: TrackDetails {
                title: Some(station.name.clone()),
                stream_url: Some(station.stream_url),
                album: None,
                track: None,
                album_id: None,
                duration: Some(0.0),
                artist: None,
                artists: vec![],
                starred: None,
                cover_art: None,
                is_stream: Some(true),
                is_podcast: None,
                is_unavailable: None,
                play_count: None,
                replay_gain: None,
            }
        }
    }
}

impl From<PodcastEpisode> for AirsonicTrack {
    fn from(episode: PodcastEpisode) -> Self {
        AirsonicTrack {
            id: episode.id.into(),
            details: TrackDetails {
                title: Some(episode.title),
                artist: Some(episode.artist),
                album: Some(episode.album),
                duration: Some(episode.duration),
                cover_art: Some(episode.cover_art),
                is_podcast: Some(true),
                album_id: None,
                starred: None,
                track: None,
                artists: vec![],
                is_stream: None,
                is_unavailable: None,
                play_count: None,
                replay_gain: None,
                stream_url: None,
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, From)]
#[serde(from = "String", into = "String")]
pub enum AirsonicTrackId {
    Track(#[from] TrackId),
    Radio(#[from] RadioId),
}

const RADIO_PREFIX: &str = "radio-";

impl From<String> for AirsonicTrackId {
    fn from(mut value: String) -> Self {
        if value.starts_with(RADIO_PREFIX) {
            value.drain(0..RADIO_PREFIX.len());
            return AirsonicTrackId::Radio(RadioId(value));
        }

        AirsonicTrackId::Track(TrackId(value))
    }
}

impl Into<String> for AirsonicTrackId {
    fn into(self) -> String {
        match self {
            AirsonicTrackId::Track(TrackId(id)) => id,
            AirsonicTrackId::Radio(RadioId(id)) => format!("{RADIO_PREFIX}{id}"),
        }
    }
}
