use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Error, Result};
use async_stream::stream;
use axum::extract::ws::{self, WebSocket};
use axum::extract::State;
use axum::Json;
use futures::{future, pin_mut};
use futures::sink::SinkExt;
use futures::stream::{SplitSink, SplitStream, Stream, StreamExt};
use serde::Serialize;
use tokio::sync::watch;
use tokio::sync::Mutex as AsyncMutex;

use crate::mpd::Mpd;
use crate::mpd::types::{MpdEvent, PlayerState};
use crate::app::Ctx;

use super::commands;

const PLAYING_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Clone, Default)]
pub struct MpdEvents {
    #[allow(unused)]
    queue: watch::Sender<()>,
    status: watch::Sender<()>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerMsg {
    Playback(PlaybackEvent),
    Queue(QueueEvent),
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

pub async fn run_websocket(ctx: Ctx, socket: WebSocket) {
    let (tx, rx) = socket.split();
    let tx = Sender::new(tx);

    let websocket_task = websocket_task(ctx.clone(), rx);
    pin_mut!(websocket_task);

    let playback_event_task = playback_event_task(ctx.clone(), tx.clone());
    pin_mut!(playback_event_task);

    let status_event_task = status_event_task(ctx.clone(), tx.clone());
    pin_mut!(status_event_task);

    let queue_event_task = queue_event_task(ctx.clone(), tx.clone());
    pin_mut!(queue_event_task);

    let result = future::select_all([
        websocket_task as Pin<&mut (dyn Future<Output = Result<()>> + Send)>,
        playback_event_task,
        status_event_task,
        queue_event_task,
    ]).await.0;

    if let Err(err) = result {
        log::error!("error running websocket: {err}");
    }
}

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

async fn playback_event_task(ctx: Ctx, tx: Sender) -> Result<()> {
    loop {
        let status = {
            let mpd = ctx.mpd.read().await;
            mpd.status().await?
        };

        let event = PlaybackEvent {
            playing: status.state == PlayerState::Play,
            position: status.elapsed.map(|s| s.0),
            duration: status.duration.map(|s| s.0),
        };

        tx.send(ServerMsg::Playback(event)).await;

        tokio::time::sleep(PLAYING_INTERVAL).await;
    }
}

async fn status_event_task(ctx: Ctx, tx: Sender) -> Result<()> {
    queue_event_common(ctx.clone(), tx, ctx.events.status.clone()).await
}

async fn queue_event_task(ctx: Ctx, tx: Sender) -> Result<()> {
    queue_event_common(ctx.clone(), tx, ctx.events.queue.clone()).await
}

async fn queue_event_common(ctx: Ctx, tx: Sender, watch: watch::Sender<()>) -> Result<()> {
    let mut watch = watch.subscribe();

    while watch.changed().await.is_ok() {
        match commands::queue(State(ctx.clone())).await {
            Ok(Json(queue)) => { tx.send(ServerMsg::Queue(QueueEvent(queue))).await; }
            Err(err) => {
                log::warn!("error fetching queue: {err}");
            }
        }
    }

    Ok(())
}

#[derive(Clone)]
pub struct Sender {
    tx: Arc<AsyncMutex<SplitSink<WebSocket, ws::Message>>>,
}

impl Sender {
    pub fn new(tx: SplitSink<WebSocket, ws::Message>) -> Self {
        Sender { tx: Arc::new(AsyncMutex::new(tx)) }
    }

    pub async fn send(&self, msg: ServerMsg) {
        if let Err(err) = self.try_send(msg).await {
            log::warn!("websocket send error: {err}");
        }
    }

    async fn try_send(&self, msg: ServerMsg) -> Result<()> {
        let json = serde_json::to_string(&msg)?;
        let msg = ws::Message::text(json);
        let mut tx = self.tx.lock().await;
        tx.send(msg).await?;
        Ok(())
    }
}

fn broken_pipe(err: &(dyn std::error::Error + 'static)) -> bool {
    io_error(err).map(io::Error::kind) == Some(io::ErrorKind::BrokenPipe)
}

fn io_error<'err>(err: &'err (dyn std::error::Error + 'static)) -> Option<&'err std::io::Error> {
    if let Some(io) = err.downcast_ref() {
        return Some(*io);
    }

    io_error(err.source()?)
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
