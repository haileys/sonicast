use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;
use futures::{future, pin_mut};
use serde::Serialize;
use tokio::sync::watch;

use crate::logging;
use crate::mpd::Mpd;
use crate::mpd::types::{MpdEvent, PlaybackState, ReplayGainMode};
use crate::app::ServerMsg;

use super::{commands, Session};

const PLAYING_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Clone, Default)]
pub struct MpdEvents {
    queue: watch::Sender<()>,
    status: watch::Sender<()>,
    options: watch::Sender<()>,
}

#[derive(Debug, Serialize)]
pub struct PlaybackEvent {
    playing: bool,
    position: Option<f64>,
    duration: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct OptionsEvent {
    volume: f64,
    repeat: bool,
    shuffle: bool,
    single: bool,
    replay_gain: ReplayGainMode,
}

#[derive(Debug, Serialize)]
pub struct QueueEvent(commands::Queue);

pub async fn run_events(session: &Session) -> Result<()> {
    let playback_event_task = playback_event_task(session);
    pin_mut!(playback_event_task);

    let status_event_task = status_event_task(session);
    pin_mut!(status_event_task);

    let queue_event_task = queue_event_task(session);
    pin_mut!(queue_event_task);

    let options_event_task = options_event_task(session);
    pin_mut!(options_event_task);

    future::select_all([
        playback_event_task as Pin<&mut (dyn Future<Output = Result<()>> + Send)>,
        status_event_task,
        queue_event_task,
        options_event_task,
    ]).await.0
}

async fn playback_event_task(session: &Session) -> Result<()> {
    loop {
        let status = {
            let mpd = session.ctx.mpd.read().await;
            mpd.status().await?
        };

        let event = PlaybackEvent {
            playing: status.state == PlaybackState::Play,
            position: status.elapsed.map(|s| s.0),
            duration: status.duration.map(|s| s.0),
        };

        session.tx.send(ServerMsg::Playback(event)).await;

        tokio::time::sleep(PLAYING_INTERVAL).await;
    }
}

async fn options_event_task(session: &Session) -> Result<()> {
    let mut watch = session.ctx.events.options.subscribe();

    loop {
        let Some(options) = get_player_options(&session).await
            .inspect_err(logging::error)
            .ok() else { continue };

        session.tx.send(ServerMsg::Options(options)).await;

        let Ok(_) = watch.changed().await else { break };
    }

    Ok(())
}

async fn get_player_options(session: &Session) -> Result<OptionsEvent> {
    let mpd = session.ctx.mpd.read().await;
    let status = mpd.status().await?;
    let replay_gain = mpd.replay_gain_status().await?;
    let volume = status.volume.unwrap_or(100) as f64 / 100.0;
    Ok(OptionsEvent {
        volume,
        shuffle: status.random,
        repeat: status.repeat,
        single: status.single,
        replay_gain,
    })
}

async fn status_event_task(session: &Session) -> Result<()> {
    queue_event_common(session, session.ctx.events.status.clone()).await
}

async fn queue_event_task(session: &Session) -> Result<()> {
    queue_event_common(session, session.ctx.events.queue.clone()).await
}

async fn queue_event_common(session: &Session, watch: watch::Sender<()>) -> Result<()> {
    let mut watch = watch.subscribe();

    while watch.changed().await.is_ok() {
        match commands::queue(session).await {
            Ok(queue) => {
                let msg = ServerMsg::Queue(QueueEvent(queue));
                session.tx.send(msg).await;
            }
            Err(err) => {
                log::warn!("error fetching queue: {err}");
            }
        }
    }

    Ok(())
}

pub async fn task(mpd: Mpd, events: MpdEvents) {
    if let Err(err) = mpd_loop(mpd, &events).await {
        panic!("mpd task: {err:?}");
    }
}

async fn mpd_loop(mpd: Mpd, events: &MpdEvents) -> Result<()> {
    let mut queue_ver = playlist_version(&mpd).await?;

    loop {
        let changed = mpd.idle().await?;
        log::debug!("mpd event: {:?}", changed);

        for event in changed.events() {
            match event {
                MpdEvent::Player => events.status.send_replace(()),
                MpdEvent::Playlist => {
                    let new_ver = playlist_version(&mpd).await?;
                    if queue_ver != new_ver {
                        queue_ver = new_ver;
                        events.queue.send_replace(());
                    }
                }
                MpdEvent::Options => events.options.send_replace(()),
                MpdEvent::Mixer => {}
            }
        }
    }
}

async fn playlist_version(mpd: &Mpd) -> Result<u32> {
    Ok(mpd.status().await?.playlist_version)
}
