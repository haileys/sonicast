use std::sync::Arc;

use anyhow::Result;
use serde::Deserialize;
use url::Url;

use crate::subsonic::{types::{CoverArtId, TrackId}, AuthParams, Subsonic, SubsonicBase};

#[derive(Clone)]
pub struct PodcastsBase {
    server: SubsonicBase,
    episode_prefix: String,
}

#[derive(Clone)]
pub struct Config {
    pub server_url: Url,
    pub episode_prefix: String,
}

impl PodcastsBase {
    pub fn new(config: &Config) -> Self {
        PodcastsBase {
            server: SubsonicBase::new(&config.server_url),
            episode_prefix: config.episode_prefix.clone(),
        }
    }

    pub async fn authenticate(&self, params: Arc<AuthParams>) -> Result<Podcasts> {
        let server = self.server.authenticate(params).await?;

        Ok(Podcasts {
            server,
            episode_prefix: self.episode_prefix.clone(),
        })
    }
}

pub struct Podcasts {
    server: Subsonic,
    episode_prefix: String,
}

impl Podcasts {
    pub fn matches(&self, id: &TrackId) -> bool {
        id.0.starts_with(&self.episode_prefix)
    }

    pub fn stream_url(&self, id: &TrackId) -> Result<Url> {
        self.server.stream_url(id)
    }

    pub fn track_id_from_stream_url(&self, url: &Url) -> Option<TrackId> {
        self.server.track_id_from_stream_url(url)
    }

    pub async fn get_podcast_episode(&self, id: &TrackId) -> Result<PodcastEpisode> {
        #[derive(Deserialize, Debug)]
        #[serde(rename_all = "camelCase")]
        pub struct GetPodcastEpisode {
            podcast_episode: PodcastEpisode,
        }

        let result = self.server.call::<GetPodcastEpisode>(
            "getPodcastEpisode", &[("id", &id.0)]
        ).await?;

        Ok(result.podcast_episode)
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PodcastEpisode {
    pub id: TrackId,
    pub title: String,
    pub album: String,
    pub artist: String,
    pub duration: f64,
    pub cover_art: CoverArtId,
}
