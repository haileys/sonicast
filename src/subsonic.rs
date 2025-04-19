use std::sync::Arc;

use reqwest::{Method, Url};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct Subsonic {
    inner: Arc<Inner>,
}

struct Inner {
    client: reqwest::Client,
    config: Config,
}

#[derive(Clone)]
pub struct Config {
    pub base_url: reqwest::Url,
    pub username: String,
    pub password: String,
    // this is not a PathBuf because we need to enforce utf-8 encoding on
    // file paths in this app. so String is the appropriate type
    #[allow(unused)]
    pub music_dir: String,
}

impl Subsonic {
    pub fn new(config: &Config) -> Self {
        Subsonic {
            inner: Arc::new(Inner {
                client: reqwest::Client::new(),
                config: config.clone(),
            }),
        }
    }

    #[allow(unused)]
    pub async fn ping(&self) -> Result<()> {
        self.call::<serde_json::Value>("ping", &[]).await?;
        Ok(())
    }

    #[allow(unused)]
    pub async fn get_random_songs(&self) -> Result<Vec<Track>> {
        let response: SubsonicResponse<RandomSongs>
            = self.call("getRandomSongs", &[]).await?;

        #[derive(Deserialize, Debug)]
        struct RandomSongs {
            #[serde(rename = "randomSongs")]
            random_songs: Tracks,
        }

        Ok(response.data.random_songs.tracks)
    }

    pub async fn get_track(&self, id: &TrackId) -> Result<Track> {
        let response: SubsonicResponse<GetSong>
            = self.call("getSong", &[("id", &id.0)]).await?;

        #[derive(Deserialize, Debug)]
        struct GetSong {
            song: Track,
        }

        Ok(response.data.song)
    }

    // pub async fn get_file_path(&self, id: &TrackId) -> Result<String> {
    //     let response: SubsonicResponse<GetSong>
    //         = self.call("getSong", &[("id", &id.0)]).await?;

    //     #[derive(Deserialize, Debug)]
    //     struct GetSong {
    //         song: Track,
    //     }

    //     let absolute_path = format!("{}{}{}",
    //         self.inner.config.music_dir,
    //         std::path::MAIN_SEPARATOR_STR,
    //         response.data.song.path,
    //     );

    //     Ok(absolute_path)
    // }

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
        let request = self.request(Method::GET, &format!("rest/{method}"))
            .query(params)
            .build()?;

        let response = self.inner.client.execute(request).await?;
        response.error_for_status_ref()?;

        Ok(response.json().await?)
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = self.inner.config.base_url.join(path).unwrap();
        self.inner.client.request(method, url)
            .query(&[
                ("u", self.inner.config.username.as_str()),
                ("p", self.inner.config.password.as_str()),
                ("f", "json"),
                ("c", "sonicast"),
                ("v", env!("CARGO_PKG_VERSION")),
            ])
    }
}

#[derive(Deserialize, Debug)]
struct SubsonicResponse<T> {
    #[serde(
        rename = "subsonic-response",
        // deserialize_with = "deserialize_subsonic_response_data",
    )]
    data: T,
}

// fn deserialize_subsonic_response_data<'de, D, T>(de: D) -> Result<T, D::Error>
//     where D: Deserializer<'de>, T: Deserialize<'de> + DeserializeOwned
// {
//     let value = serde_json::Value::deserialize(de)?;

//     match serde_json::from_value(value) {
//         Ok(value) => Ok(value),
//         Err(err) => {
//             log::error!("failed to deserialize {}", std::any::type_name::<T>())
//         }
//     }
// }

#[derive(Deserialize, Debug)]
struct Tracks {
    #[serde(rename = "song")]
    tracks: Vec<Track>,
    // #[serde(flatten)]
    // tracks: serde_json::Value,
}

// #[derive(Deserialize)]
// struct Song {
//     song: Track,
// }

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
