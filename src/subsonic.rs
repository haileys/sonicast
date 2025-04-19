use std::sync::Arc;

use reqwest::{Method, Url};
use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct SubsonicBase {
    inner: Arc<Inner>,
}

struct Inner {
    client: reqwest::Client,
    base_url: reqwest::Url,
}

#[derive(Clone)]
pub struct Config {
    pub base_url: reqwest::Url,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AuthParams {
    #[serde(rename = "u")]
    username: Option<String>,
    #[serde(rename = "s")]
    salt: Option<String>,
    #[serde(rename = "t")]
    token: Option<String>,
    #[serde(rename = "p")]
    password: Option<String>,
}

impl SubsonicBase {
    pub fn new(config: &Config) -> Self {
        SubsonicBase {
            inner: Arc::new(Inner {
                client: reqwest::Client::new(),
                base_url: config.base_url.clone(),
            }),
        }
    }

    pub async fn authenticate(&self, params: AuthParams) -> Result<Subsonic> {
        let subsonic = Subsonic {
            inner: self.inner.clone(),
            auth: Arc::new(params),
        };

        // test auth details:
        subsonic.ping().await?;

        Ok(subsonic)
    }
}

pub struct Subsonic {
    inner: Arc<Inner>,
    auth: Arc<AuthParams>,
}

impl Subsonic {
    #[allow(unused)]
    pub async fn ping(&self) -> Result<()> {
        self.call::<serde_json::Value>("ping", &[]).await?;
        Ok(())
    }

    #[allow(unused)]
    pub async fn get_random_songs(&self) -> Result<Vec<Track>> {
        #[derive(Deserialize, Debug)]
        struct RandomSongs {
            #[serde(rename = "randomSongs")]
            random_songs: Tracks,
        }

        Ok(self.call::<RandomSongs>("getRandomSongs", &[])
            .await?
            .random_songs
            .tracks)
    }

    pub async fn get_track(&self, id: &TrackId) -> Result<Track> {
        #[derive(Deserialize, Debug)]
        struct GetSong {
            song: Track,
        }

        Ok(self.call::<GetSong>("getSong", &[("id", &id.0)])
            .await?
            .song)
    }

    pub fn stream_url(&self, id: &TrackId) -> Result<Url> {
        let req = self
            .request(Method::GET, "rest/stream")
            .query(&[("id", &id.0)]);

        Ok(req.build()?.url().clone())
    }

    pub fn track_id_from_stream_url(&self, url: &Url) -> Option<TrackId> {
        url.query_pairs()
            .find(|(name, _)| name == "id")
            .map(|(_, value)| TrackId(value.to_string()))
    }

    async fn call<T>(&self, method: &str, params: &[(&str, &str)]) -> Result<T>
        where T: serde::de::DeserializeOwned
    {
        #[derive(Deserialize, Debug)]
        struct RootResponse<T> {
            #[serde(rename = "subsonic-response")]
            response: SubsonicResponse<T>
        }

        #[derive(Deserialize, Debug)]
        #[serde(untagged)]
        enum SubsonicResponse<T> {
            Ok(T),
            Err { error: SubsonicError }
        }

        #[derive(Deserialize, Debug)]
        struct SubsonicError {
            message: String,
        }

        let request = self.request(Method::GET, &format!("rest/{method}"))
            .query(params)
            .build()?;

        let response = self.inner.client.execute(request).await?;
        response.error_for_status_ref()?;

        let text = response.text().await?;

        let root = serde_json::from_str::<RootResponse<T>>(&text)
            .map_err(anyhow::Error::from)
            .with_context(|| {
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(json) => format!("original json: {json:#?}"),
                    Err(_) => format!("original text: {text:?}"),
                }
            })?;

        match root.response {
            SubsonicResponse::Ok(data) => Ok(data),
            SubsonicResponse::Err { error } => {
                anyhow::bail!("subsonic error: {}", error.message);
            }
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = self.inner.base_url.join(path).unwrap();

        self.inner.client.request(method, url)
            .query(&*self.auth)
            .query(&[
                ("f", "json"),
                ("c", "sonicast"),
                ("v", env!("CARGO_PKG_VERSION")),
            ])
    }
}

#[derive(Deserialize, Debug)]
struct Tracks {
    #[serde(rename = "song")]
    tracks: Vec<Track>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Track {
    pub id: TrackId,
    pub artist: Option<String>,
    pub title: Option<String>,
    pub duration: Option<f64>,
    pub starred: Option<bool>,
    #[serde(rename = "coverArt")]
    pub cover_art: Option<CoverArtId>,
    pub track: Option<usize>,
    pub album: Option<String>,
    #[serde(rename = "albumId")]
    pub album_id: Option<AlbumId>,
    pub artists: Vec<TrackArtist>,
    #[serde(rename = "isStream")]
    pub is_stream: Option<bool>,
    #[serde(rename = "isPodcast")]
    pub is_podcast: Option<bool>,
    #[serde(rename = "isUnavailable")]
    pub is_unavailable: Option<bool>,
    #[serde(rename = "playCount")]
    pub play_count: Option<usize>,
    #[serde(rename = "replayGain")]
    pub replay_gain: Option<serde_json::Value>,
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

#[derive(Deserialize, Serialize, Debug)]
pub struct TrackId(pub String);

#[derive(Deserialize, Serialize, Debug)]
pub struct AlbumId(pub String);

#[derive(Deserialize, Serialize, Debug)]
pub struct ArtistId(pub String);

#[derive(Deserialize, Serialize, Debug)]
pub struct CoverArtId(pub String);
