use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};

use crate::player::{Session, Command, helper};
use crate::mpd::types::{PlaybackState, Seconds};
use crate::mpd::{self, Mpd};

use super::types::{AirsonicTrack, AirsonicTrackId};
use super::{Response, ServerMsg};

macro_rules! commands {
    { $( $variant:ident : $func:ident ( $( $param:ty )? ) => $result:ty ; )* } => {
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "kebab-case", tag = "name", content = "param")]
        pub enum CommandKind {
            $( $variant $( ( $param ) )?, )*
        }

        #[derive(Debug, Serialize)]
        #[serde(rename_all = "kebab-case", tag = "kind", content = "data")]
        pub enum ResponseKind {
            Error { message: String },
            $( $variant ( $result ), )*
        }

        async fn dispatch_kind(session: &Session, command: CommandKind) -> Result<ResponseKind> {
            let command_name;
            let result = match command {
                $(
                    CommandKind::$variant $( ( commands!{@param_var param: $param} ) )? => {
                        command_name = stringify!($variant);
                        $func(session $(, commands!{@param_var param: $param} )? ).await
                            .map(ResponseKind::$variant)
                    }
                )*
            };
            result.with_context(|| format!("dispatching command {command_name}"))
        }
    };

    // special internal rule to allow for $()? expansions of param
    // without including $param in macro output
    { @param_var $param_ident:ident : $param_ty:ty } => { $param_ident };
}

pub async fn dispatch(session: &Session, command: Command) {
    let kind = match dispatch_kind(session, command.kind).await {
        Ok(kind) => kind,
        Err(err) => {
            log::error!("{err:?}");
            ResponseKind::Error { message: format!("{err}") }
        }
    };

    let response = Response { seq: command.seq, kind };
    session.tx.send(ServerMsg::Response(response)).await;
}

commands! {
    Play: play() => ();
    Pause: pause() => ();
    Stop: stop() => ();
    SkipNext: skip_next() => ();
    SkipPrevious: skip_previous() => ();
    Seek: seek(Seek) => ();
    PlayIndex: play_index(PlayIndex) => ();
    ResetQueue: reset_queue() => ();
    ClearQueue: clear_queue() => ();
    AddToQueue: add_to_queue(AddToQueue) => ();
    SetNextInQueue: set_next_in_queue(AddToQueue) => ();
    Queue: queue() => Queue;
    PlayTrackList: play_track_list(PlayTrackList) => ();
    LoadPlayerState: load_player_state(PlayerState) => ();
    UnloadPlayerState: unload_player_state() => PlayerState;
    RemoveFromQueue: remove_from_queue(RemoveFromQueue) => ();
    ShuffleQueue: shuffle_queue() => ();
    ReplayGainMode: replay_gain_mode(ReplayGainMode) => ();
    SetRepeat: set_repeat(SetRepeat) => ();
    SetShuffle: set_shuffle(SetShuffle) => ();
    SetVolume: set_volume(SetVolume) => ();
    SetPlaybackRate: set_playback_rate(SetPlaybackRate) => ();
}

async fn play(session: &Session) -> Result<()> {
    let mpd = session.mpd().await;
    mpd.play().await
}

async fn pause(session: &Session) -> Result<()> {
    let mpd = session.mpd().await;
    mpd.pause().await
}

async fn stop(session: &Session) -> Result<()> {
    let mpd = session.mpd().await;
    mpd.stop().await
}

async fn skip_next(session: &Session) -> Result<()> {
    let mut mpd = session.mpd().await;
    player_op(&mut mpd, Op::Next).await
}

async fn skip_previous(session: &Session) -> Result<()> {
    let mut mpd = session.mpd().await;
    player_op(&mut mpd, Op::Previous).await
}

#[derive(Debug, Deserialize)]
pub struct Seek {
    #[serde(rename = "pos")]
    position: f64,
}

async fn seek(session: &Session, param: Seek) -> Result<()> {
    let mut mpd = session.mpd().await;
    player_op(&mut mpd, Op::Seek(param.position)).await
}

#[derive(Debug, Deserialize)]
pub struct PlayIndex {
    index: usize,
}

async fn play_index(session: &Session, param: PlayIndex) -> Result<()> {
    let mpd = session.mpd().await;
    mpd.playpos(param.index).await
}

async fn reset_queue(session: &Session) -> Result<()> {
    session.mpd().await.stop().await
}

async fn clear_queue(session: &Session) -> Result<()> {
    session.mpd().await.clear().await
}

#[derive(Deserialize, Debug)]
pub struct AddToQueue {
    tracks: Vec<AirsonicTrackId>,
}

async fn add_to_queue(session: &Session, params: AddToQueue) -> Result<()> {
    let resolver = session.resolver();
    let track_urls = resolver.stream_urls_for(&params.tracks).await?;

    let mpd = session.mpd().await;
    for url in &track_urls {
        mpd.addid(url.as_str()).await?;
    }

    Ok(())
}

async fn set_next_in_queue(session: &Session, params: AddToQueue) -> Result<()> {
    let resolver = session.resolver();
    let track_urls = resolver.stream_urls_for(&params.tracks).await?;

    let mut mpd = session.mpd().await;
    helper::atomic_enqueue_tracks(&mut mpd, &track_urls, Some(0)).await?;

    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    tracks: Vec<AirsonicTrack>,
    current_track: Option<usize>,
    current_track_position: Option<f64>,
}

pub async fn queue(session: &Session) -> Result<Queue> {
    let mpd = session.mpd().await;
    let queue = mpd.playlistinfo().await?;
    let status = mpd.status().await?;
    drop(mpd);

    let resolver = session.resolver();
    let tracks = resolver.load_tracks_for(&queue.items).await?;

    let current_track = queue.items.iter()
        .position(|item| Some(&item.id) == status.song_id.as_ref());

    let current_track_position = status.elapsed.map(|sec| sec.0);

    Ok(Queue {
        tracks,
        current_track,
        current_track_position,
    })
}

#[derive(Deserialize, Serialize, Debug)]
pub struct PlayerState {
    tracks: Vec<AirsonicTrack>,
    index: usize,
    time: f64,
    shuffle: bool,
    repeat: bool,
    playing: bool,
}

// loads entire player state, used for switching to this player from another
async fn load_player_state(session: &Session, params: PlayerState) -> Result<()> {
    let resolver = session.resolver();

    let track_ids = params.tracks.iter()
        .map(|t| t.id.clone())
        .collect::<Vec<_>>();

    let track_urls = resolver.stream_urls_for(&track_ids).await?;

    let mpd = session.mpd().await;
    mpd.clear().await?;

    for url in &track_urls {
        mpd.addid(url.as_str()).await?;
    }

    mpd.seek(params.index, params.time).await?;
    mpd.random(params.shuffle).await?;
    mpd.repeat(params.repeat).await?;

    if params.playing {
        mpd.play().await?;
    }

    Ok(())
}

// dumps player state, stops, clears queue; used for switching away from this player
async fn unload_player_state(session: &Session) -> Result<PlayerState> {
    let mpd = session.mpd().await;
    let status = mpd.status().await?;
    let queue = mpd.playlistinfo().await?;
    mpd.stop().await?;
    mpd.clear().await?;
    drop(mpd);

    let resolver = session.resolver();
    let tracks = resolver.load_tracks_for(&queue.items).await?;

    Ok(PlayerState {
        tracks,
        index: status.song.unwrap_or_default(),
        time: status.elapsed.map(|Seconds(s)| s).unwrap_or_default(),
        shuffle: status.random,
        repeat: status.repeat,
        playing: status.state == PlaybackState::Play,
    })
}

#[derive(Deserialize, Debug)]
pub struct PlayTrackList {
    tracks: Vec<AirsonicTrackId>,
    index: Option<usize>,
    shuffle: Option<bool>,
}

async fn play_track_list(session: &Session, params: PlayTrackList) -> Result<()> {
    let resolver = session.resolver();
    let track_urls = resolver.stream_urls_for(&params.tracks).await?;

    let mpd = session.mpd().await;

    // first clear the playlist
    mpd.clear().await?;

    // set shuffle if it was requested
    if let Some(shuffle) = params.shuffle {
        mpd.random(shuffle).await?;
    }

    // add all tracks in the same order as they were provided
    for url in &track_urls {
        mpd.addid(url.as_str()).await?;
    }

    // then play, from index if given
    if let Some(index) = params.index {
        mpd.playpos(index).await?;
    } else {
        mpd.play().await?;
    }

    Ok(())
}

#[derive(Deserialize, Debug)]
pub struct RemoveFromQueue {
    index: usize,
}

async fn remove_from_queue(session: &Session, params: RemoveFromQueue) -> Result<()> {
    let mpd = session.mpd().await;

    if let Some(pos) = isize::try_from(params.index).ok() {
        mpd.delete(pos).await?;
    }

    Ok(())
}

async fn shuffle_queue(session: &Session) -> Result<()> {
    session.mpd().await.shuffle().await
}

#[derive(Deserialize, Debug)]
pub struct ReplayGainMode {
    mode: mpd::types::ReplayGainMode,
}

async fn replay_gain_mode(session: &Session, params: ReplayGainMode) -> Result<()> {
    session.mpd().await.replay_gain_mode(params.mode).await
}

#[derive(Deserialize, Debug)]
pub struct SetRepeat {
    repeat: bool,
}

async fn set_repeat(session: &Session, params: SetRepeat) -> Result<()> {
    session.mpd().await.repeat(params.repeat).await
}

#[derive(Deserialize, Debug)]
pub struct SetShuffle {
    shuffle: bool,
}

async fn set_shuffle(session: &Session, params: SetShuffle) -> Result<()> {
    session.mpd().await.random(params.shuffle).await
}

#[derive(Deserialize, Debug)]
pub struct SetVolume {
    #[allow(unused)]
    volume: f64
}

async fn set_volume(session: &Session, params: SetVolume) -> Result<()> {
    // convert from 0-1 airsonic volume to 0-100 mpd volume:
    let volume = (params.volume * 100.0).round() as usize;
    session.mpd().await.setvol(volume).await
}

#[derive(Deserialize, Debug)]
pub struct SetPlaybackRate {
    #[allow(unused)]
    rate: f64
}

async fn set_playback_rate(_session: &Session, _params: SetPlaybackRate) -> Result<()> {
    anyhow::bail!("set-playback-rate not currently implemented on mpd");
}

enum Op {
    Next,
    Previous,
    Seek(f64),
}

// this function is necessary to work around some weird mpd bug where on
// next/previous/seek etc it winds up stuck, despite showing state = play
async fn player_op(mpd: &mut Mpd, op: Op) -> anyhow::Result<()> {
    let state = mpd.status().await?.state;
    mpd.pause().await?;

    match op {
        Op::Next => { mpd.next().await? }
        Op::Previous => { mpd.previous().await? }
        Op::Seek(pos) => { mpd.seekcur(pos).await? }
    }

    if state == PlaybackState::Play {
        mpd.play().await?;
    }

    Ok(())
}
