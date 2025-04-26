use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Deserialize, Serialize, Debug)]
pub struct Track {
    pub id: TrackId,
    #[serde(flatten)]
    pub details: TrackDetails,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct TrackDetails {
    pub artist: Option<String>,
    pub title: Option<String>,
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starred: Option<String>,
    #[serde(rename = "coverArt", skip_serializing_if = "Option::is_none")]
    pub cover_art: Option<CoverArtId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(rename = "albumId")]
    pub album_id: Option<AlbumId>,
    pub artists: Vec<TrackArtist>,
    #[serde(rename = "isStream", skip_serializing_if = "Option::is_none")]
    pub is_stream: Option<bool>,
    #[serde(rename = "isPodcast", skip_serializing_if = "Option::is_none")]
    pub is_podcast: Option<bool>,
    #[serde(rename = "isUnavailable", skip_serializing_if = "Option::is_none")]
    pub is_unavailable: Option<bool>,
    #[serde(rename = "playCount", skip_serializing_if = "Option::is_none")]
    pub play_count: Option<usize>,
    #[serde(rename = "replayGain", skip_serializing_if = "Option::is_none")]
    pub replay_gain: Option<serde_json::Value>,
    #[serde(rename = "streamUrl", skip_serializing_if = "Option::is_none")]
    pub stream_url: Option<Url>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct TrackArtist {
    pub name: String,
    pub id: ArtistId,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ReplayGain {
    #[serde(rename = "trackGain")]
    pub track_gain: Option<f64>,
    #[serde(rename = "trackPeak")]
    pub track_peak: Option<f64>,
    #[serde(rename = "albumGain")]
    pub album_gain: Option<f64>,
    #[serde(rename = "albumPeak")]
    pub album_peak: Option<f64>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TrackId(pub String);

#[derive(Deserialize, Serialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct RadioId(pub String);

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct AlbumId(pub String);

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ArtistId(pub String);

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct CoverArtId(pub String);

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct RadioStation {
    pub id: RadioId,
    pub name: String,
    #[serde(rename = "streamUrl")]
    pub stream_url: Url,
    #[serde(rename = "homePageUrl")]
    pub homepage_url: String,
}
