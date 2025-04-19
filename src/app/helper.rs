use std::collections::HashMap;

use anyhow::Result;
use futures::stream::{FuturesOrdered, TryStreamExt};
use tokio::sync::OnceCell;
use url::Url;

use crate::mpd::Mpd;
use crate::subsonic::Subsonic;
use crate::subsonic::types::{RadioId, RadioStation};

use super::types::{AirsonicTrack, AirsonicTrackId};

async fn gather<T>(iter: impl Iterator<Item = impl Future<Output = Result<T>>>) -> Result<Vec<T>> {
    iter.collect::<FuturesOrdered<_>>()
        .try_collect()
        .await
}

type RadioStationMap = HashMap<RadioId, RadioStation>;

pub struct Resolver<'a> {
    subsonic: &'a Subsonic,
    stations: OnceCell<RadioStationMap>,
}

impl<'a> Resolver<'a> {
    pub fn new(subsonic: &'a Subsonic) -> Self {
        Resolver { subsonic, stations: Default::default() }
    }

    pub async fn stream_urls_for(&self, ids: &[AirsonicTrackId]) -> Result<Vec<Url>> {
        let futs = ids.iter()
            .map(|id| self.stream_url_for_id(id));

        gather(futs).await
    }

    pub async fn stream_url_for_id(&self, id: &AirsonicTrackId) -> Result<Url> {
        match id {
            AirsonicTrackId::Track(id) => {
                Ok(self.subsonic.stream_url(id)?)
            }
            AirsonicTrackId::Radio(id) => {
                let station = self.resolve_radio_id(id).await?;
                Ok(station.stream_url.clone())
            }
        }
    }

    pub async fn load_tracks_for(&self, urls: &[Url]) -> Result<Vec<AirsonicTrack>> {
        let futs = urls.iter()
            .map(|url| self.load_track_for_url(url));

        gather(futs).await
    }

    pub async fn load_track_for_url(&self, url: &Url) -> Result<AirsonicTrack> {
        if let Some(id) = self.subsonic.track_id_from_stream_url(&url) {
            let track = self.subsonic.get_track(&id).await?;
            return Ok(track.into());
        }

        if let Some(station) = self.resolve_radio_url(url).await? {
            return Ok(station.into());
        }

        anyhow::bail!("could not resolve url: {url}")
    }

    async fn radio_stations(&self) -> Result<&RadioStationMap> {
        self.stations.get_or_try_init(|| async {
            let stations = self.subsonic.get_radio_stations().await?;
            Ok(stations.into_iter()
                .map(|station| (station.id.clone(), station))
                .collect())
        }).await
    }

    async fn resolve_radio_id(&self, id: &RadioId) -> Result<RadioStation> {
        let stations = self.radio_stations().await?;
        stations.get(id).cloned().ok_or_else(||
            anyhow::format_err!("radio station not found: {id:?}"))
    }

    async fn resolve_radio_url(&self, url: &Url) -> Result<Option<RadioStation>> {
        let stations = self.radio_stations().await?;
        Ok(stations.values().find(|station| &station.stream_url == url).cloned())
    }
}

pub async fn atomic_enqueue_tracks(mpd: &mut Mpd, urls: &[Url], position: Option<isize>) -> Result<()> {
    const PLAYLIST_NAME: &str = "_sonicast_atomic_queue";
    mpd.playlistclear(PLAYLIST_NAME).await?;

    for url in urls {
        mpd.playlistadd(PLAYLIST_NAME, url.as_str()).await?;
    }

    mpd.load(PLAYLIST_NAME, None, position).await?;
    Ok(())
}
