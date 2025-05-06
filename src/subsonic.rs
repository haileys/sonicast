use std::sync::Arc;

use derive_more::Display;
use reqwest::{Method, Url};
use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod types;
use types::{Track, TrackId, RadioStation};

#[derive(Clone)]
pub struct SubsonicBase {
    inner: Arc<Inner>,
}

struct Inner {
    client: reqwest::Client,
    base_url: reqwest::Url,
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
    pub fn new(base_url: &Url) -> Self {
        SubsonicBase {
            inner: Arc::new(Inner {
                client: reqwest::Client::new(),
                base_url: base_url.clone(),
            }),
        }
    }

    pub async fn authenticate(&self, params: Arc<AuthParams>) -> Result<Subsonic> {
        let subsonic = Subsonic {
            inner: self.inner.clone(),
            auth: params,
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

#[derive(Deserialize, Debug, Error)]
#[error("subsonic error {code}: {message}")]
pub struct SubsonicError {
    code: SubsonicErrorCode,
    message: String,
}

#[derive(Debug, Deserialize, Serialize, Display)]
#[serde(from = "usize")]
pub enum SubsonicErrorCode {
    NotFound,
    Other(usize),
}

impl From<usize> for SubsonicErrorCode {
    fn from(code: usize) -> Self {
        match code {
            70 => SubsonicErrorCode::NotFound,
            _ => SubsonicErrorCode::Other(code),
        }
    }
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

        #[derive(Deserialize, Debug)]
        struct Tracks {
            #[serde(rename = "song")]
            tracks: Vec<Track>,
        }

        Ok(self.call::<RandomSongs>("getRandomSongs", &[])
            .await?
            .random_songs
            .tracks)
    }

    pub fn base_url(&self) -> &Url {
        &self.inner.base_url
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

    pub async fn get_radio_stations(&self) -> Result<Vec<RadioStation>> {
        #[derive(Deserialize, Debug)]
        struct Stations {
            #[serde(rename = "internetRadioStations")]
            stations: Station,
        }

        #[derive(Deserialize, Debug)]
        struct Station {
            #[serde(rename = "internetRadioStation")]
            station: Vec<RadioStation>,
        }

        Ok(self.call::<Stations>("getInternetRadioStations", &[])
            .await?
            .stations
            .station)
    }

    pub fn stream_url(&self, id: &TrackId) -> Result<Url> {
        let req = self
            .request(Method::GET, "rest/stream")
            .query(&[("id", &id.0)]);

        Ok(req.build()?.url().clone())
    }

    pub fn track_id_from_stream_url(&self, url: &Url) -> Option<TrackId> {
        if self.base_url().origin() != url.origin() {
            return None;
        }

        url.query_pairs()
            .find(|(name, _)| name == "id")
            .map(|(_, value)| TrackId(value.to_string()))
    }

    pub async fn call<T>(&self, method: &str, params: &[(&str, &str)]) -> Result<T>
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
            SubsonicResponse::Err { error } => Err(error.into()),
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
