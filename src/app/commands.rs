use anyhow::{Result, Context};
use axum::extract::{Json, State};
use serde::{Deserialize, Serialize};

use crate::app::{AppResult, Ctx, Session, Command, helper};
use crate::mpd::{self, Mpd};
use crate::mpd::types::PlayerState;
use crate::subsonic::{Track, TrackId};

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
            log::error!("dispatching command: {err}");
            ResponseKind::Error { message: format!("{err}") }
        }
    };

    let response = Response { seq: command.seq, kind };
    session.tx.send(ServerMsg::Response(response)).await;
}

commands! {
    Play: play() => ();
    Pause: pause() => ();
    SkipNext: skip_next() => ();
    SkipPrevious: skip_previous() => ();
    Seek: seek(Seek) => ();
    PlayIndex: play_index(PlayIndex) => ();
}

async fn play(session: &Session) -> Result<()> {
    let mpd = session.mpd().await;
    mpd.play().await
}

async fn pause(session: &Session) -> Result<()> {
    let mpd = session.mpd().await;
    mpd.pause().await
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

    if state == PlayerState::Play {
        mpd.play().await?;
    }

    Ok(())
}

// #[derive(Debug, Deserialize)]
// #[serde(rename_all = "kebab-case", tag = "cmd", content = "arg")]
// pub enum CommandKind {
//     Play,
//     Pause,
//     SkipNext,
//     SkipPrevious,
// }

// #[derive(Debug, Serialize)]
// #[serde(rename_all = "kebab-case", tag = "cmd", content = "data")]
// pub enum ResponseKind {
//     Play(()),
//     Pause(()),
//     SkipNext(()),
//     SkipPrevious(()),
// }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    tracks: Vec<Track>,
    current_track: Option<usize>,
    current_track_position: Option<f64>,
}

pub async fn queue(ctx: State<Ctx>) -> AppResult<Json<Queue>> {
    let mpd = ctx.mpd.read().await;
    let queue = mpd.playlistinfo().await?;
    let status = mpd.status().await?;
    drop(mpd);

    let urls = queue.items.iter()
        .map(|item| item.file.as_str())
        .collect::<Vec<_>>();

    let tracks = helper::load_tracks_for_urls(&ctx.subsonic, &urls).await?;

    let current_track = queue.items.iter()
        .position(|item| Some(&item.id) == status.song_id.as_ref());

    let current_track_position = status.elapsed.map(|sec| sec.0);

    Ok(Json(Queue {
        tracks,
        current_track,
        current_track_position,
    }))
}

#[derive(Deserialize)]
pub struct PlayTrackListParams {
    tracks: Vec<TrackId>,
    index: Option<usize>,
    shuffle: Option<bool>,
}

pub async fn play_track_list(ctx: State<Ctx>, params: Json<PlayTrackListParams>) -> AppResult<()> {
    let track_urls = helper::track_urls(&ctx.subsonic, &params.tracks)?;

    let mpd = ctx.mpd.write().await;

    // first clear the playlist
    mpd.clear().await?;

    // set shuffle if it was requested
    if let Some(shuffle) = params.shuffle {
        mpd.random(shuffle).await?;
    }

    // add all tracks in the same order as they were provided
    for url in &track_urls {
        mpd.addid(url).await?;
    }

    // then play, from index if given
    if let Some(index) = params.index {
        mpd.playpos(index).await?;
    } else {
        mpd.play().await?;
    }

    Ok(())
}

pub async fn reset_queue(ctx: State<Ctx>) -> AppResult<()> {
    let mpd = ctx.mpd.write().await;
    mpd.stop().await?;
    Ok(())
}

pub async fn clear_queue(ctx: State<Ctx>) -> AppResult<()> {
    let mpd = ctx.mpd.write().await;
    mpd.clear().await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct AddToQueueParams {
    tracks: Vec<TrackId>,
}

pub async fn add_to_queue(ctx: State<Ctx>, params: Json<AddToQueueParams>) -> AppResult<()> {
    let track_paths = helper::track_urls(&ctx.subsonic, &params.tracks)?;

    let mpd = ctx.mpd.write().await;
    for path in &track_paths {
        mpd.addid(path).await?;
    }

    Ok(())
}

pub async fn set_next_in_queue(ctx: State<Ctx>, params: Json<AddToQueueParams>) -> AppResult<()> {
    let track_paths = helper::track_urls(&ctx.subsonic, &params.tracks)?;

    let mut mpd = ctx.mpd.write().await;
    helper::atomic_enqueue_tracks(&mut mpd, &track_paths, Some(0)).await?;

    Ok(())
}

#[derive(Deserialize)]
pub struct RemoveFromQueueParams {
    index: usize,
}

pub async fn remove_from_queue(ctx: State<Ctx>, params: Json<RemoveFromQueueParams>) -> AppResult<()> {
    let mpd = ctx.mpd.write().await;

    if let Some(pos) = isize::try_from(params.index).ok() {
        mpd.delete(pos).await?;
    }

    Ok(())
}

pub async fn shuffle_queue(ctx: State<Ctx>) -> AppResult<()> {
    let mpd = ctx.mpd.write().await;
    mpd.shuffle().await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct ReplayGainModeParams {
    mode: mpd::ReplayGainMode,
}

pub async fn replay_gain_mode(ctx: State<Ctx>, params: Json<ReplayGainModeParams>) -> AppResult<()> {
    let mpd = ctx.mpd.write().await;
    mpd.replay_gain_mode(params.mode).await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct SetRepeatParams {
    repeat: bool,
}

pub async fn set_repeat(ctx: State<Ctx>, params: Json<SetRepeatParams>) -> AppResult<()> {
    let mpd = ctx.mpd.write().await;
    mpd.repeat(params.repeat).await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct SetShuffleParams {
    shuffle: bool,
}

pub async fn set_shuffle(ctx: State<Ctx>, params: Json<SetShuffleParams>) -> AppResult<()> {
    let mpd = ctx.mpd.write().await;
    mpd.random(params.shuffle).await?;
    Ok(())
}

#[derive(Deserialize, Debug)]
pub struct SetVolumeParams {
    #[allow(unused)]
    volume: f64
}

pub async fn set_volume(_ctx: State<Ctx>, params: Json<SetVolumeParams>) -> AppResult<()> {
    todo!("set-volume: {params:?}");
}

#[derive(Deserialize, Debug)]
pub struct SetPlaybackRateParams {
    #[allow(unused)]
    rate: f64
}

pub async fn set_playback_rate(_ctx: State<Ctx>, params: Json<SetPlaybackRateParams>) -> AppResult<()> {
    todo!("set-playback-rate: {params:?}");
}
