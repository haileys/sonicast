use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;
use futures::{future, pin_mut};
use serde::Serialize;
use tokio::sync::watch;

use crate::mpd::Mpd;
use crate::mpd::types::{MpdEvent, PlayerState};
use crate::app::ServerMsg;

use super::{commands, Session};

const PLAYING_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Clone, Default)]
pub struct MpdEvents {
    #[allow(unused)]
    queue: watch::Sender<()>,
    status: watch::Sender<()>,
}

#[derive(Debug, Serialize)]
pub struct PlaybackEvent {
    playing: bool,
    position: Option<f64>,
    duration: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct StatusEvent {

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

    future::select_all([
        // websocket_task,
        playback_event_task as Pin<&mut (dyn Future<Output = Result<()>> + Send)>,
        status_event_task,
        queue_event_task,
    ]).await.0
}

/*
async fn websocket_task(ctx: Ctx, rx: SplitStream<WebSocket>) -> Result<()> {
    pin_mut!(rx);

    while let Some(item) = rx.next().await {
        let msg = match item {
            Ok(msg) => msg,
            Err(err) if broken_pipe(&err) => { break },
            Err(err) => {
                log::warn!("websocket error: {err:?}");
                break;
            }
        };

        let ws::Message::Text(_text) = msg else { continue };
    }

    Ok(())
}
*/

async fn playback_event_task(session: &Session) -> Result<()> {
    loop {
        let status = {
            let mpd = session.ctx.mpd.read().await;
            mpd.status().await?
        };

        let event = PlaybackEvent {
            playing: status.state == PlayerState::Play,
            position: status.elapsed.map(|s| s.0),
            duration: status.duration.map(|s| s.0),
        };

        session.tx.send(ServerMsg::Playback(event)).await;

        tokio::time::sleep(PLAYING_INTERVAL).await;
    }
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
    let mut ver = playlist_version(&mpd).await?;

    loop {
        let changed = mpd.idle().await?;

        for event in changed.events() {
            match event {
                MpdEvent::Player => events.status.send_replace(()),
                MpdEvent::Playlist => {
                    log::info!("mpd playlist event!");
                    let new_ver = playlist_version(&mpd).await?;
                    if ver != new_ver {
                        log::info!("playlist version changed: from {ver} => to {new_ver}");
                        ver = new_ver;
                    }
                }
            }
        }
    }
}

async fn playlist_version(mpd: &Mpd) -> Result<u32> {
    Ok(mpd.status().await?.playlist_version)
}
