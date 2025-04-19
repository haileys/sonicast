use anyhow::{Context, Result};
use futures::stream::{FuturesOrdered, TryStreamExt};
use url::Url;

use crate::mpd::Mpd;
use crate::subsonic::{Subsonic, Track, TrackId};

async fn gather<T>(iter: impl Iterator<Item = impl Future<Output = Result<T>>>) -> Result<Vec<T>> {
    iter.collect::<FuturesOrdered<_>>()
        .try_collect()
        .await
}

// pub async fn load_track_paths(subsonic: &Subsonic, track_ids: &[TrackId]) -> Result<Vec<String>> {
//     gather(track_ids
//         .iter()
//         .map(|id| subsonic.get_file_path(id))).await
// }

pub fn track_urls(subsonic: &Subsonic, track_ids: &[TrackId]) -> Result<Vec<String>> {
    Ok(track_ids.iter()
        .map(|id| subsonic.stream_url(id).map(|url| url.to_string()))
        .collect::<Result<Vec<_>, _>>()?)
}

pub async fn load_tracks_for_urls(subsonic: &Subsonic, urls: &[&str]) -> Result<Vec<Track>> {
    gather(urls.iter()
        .map(async |url| {
            load_track_for_url(subsonic, url).await
                .with_context(|| "loading track for url: {url}")
        })).await
}

async fn load_track_for_url(subsonic: &Subsonic, url: &str) -> Result<Track> {
    let url = Url::parse(url)?;
    let Some(id) = subsonic.track_id_from_stream_url(&url) else {
        anyhow::bail!("no track id in url: {url}");
    };
    Ok(subsonic.get_track(&id).await
        .with_context(|| format!("{id:?}"))?)
}

pub async fn atomic_enqueue_tracks(mpd: &mut Mpd, locations: &[String], position: Option<isize>) -> Result<()> {
    const PLAYLIST_NAME: &str = "_sonicast_atomic_queue";
    mpd.playlistclear(PLAYLIST_NAME).await?;

    for location in locations {
        mpd.playlistadd(PLAYLIST_NAME, location).await?;
    }

    mpd.load(PLAYLIST_NAME, None, position).await?;
    Ok(())
}
