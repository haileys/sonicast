use std::sync::Arc;

use crate::mpd::{self, Mpd};
use crate::subsonic::{self, Subsonic};
use crate::util::broken_pipe;

use anyhow::Result;
use async_stream::stream;
use axum::extract::State;
use axum::extract::ws::{self, WebSocket, WebSocketUpgrade};
use axum::http::{Method, StatusCode};
use axum::response::IntoResponse;
use futures::{future, Stream};
use futures::sink::SinkExt;
use futures::stream::{SplitSink, SplitStream};
use futures::{pin_mut, StreamExt};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{RwLock, RwLockWriteGuard, Mutex as AsyncMutex};
use tower_http::cors::{Any, CorsLayer};
use tower::ServiceBuilder;

mod helper;
mod events;
mod commands;

pub struct Config {
    pub subsonic: subsonic::Config,
    pub mpd: mpd::Config,
}

pub async fn run(config: &Config) -> Result<()> {
    use axum::Router;
    use axum::routing::{get, post};

    // open clients, including two mpd connections
    //  - one for commands, the other for events
    let subsonic = Subsonic::new(&config.subsonic);
    let mpd = Mpd::connect(&config.mpd).await?;
    let mpd_event = Mpd::connect(&config.mpd).await?;

    let mpd = Arc::new(RwLock::new(mpd));
    let ctx = Ctx::new(CtxData {
        subsonic,
        mpd,
        events: events::MpdEvents::default(),
    });

    // spawn mpd event task
    tokio::task::spawn(events::task(mpd_event, ctx.events.clone()));

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST])
        .allow_origin(Any)
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    let app = Router::new()
        .route("/ws", get(websocket))
        .route("/queue", get(commands::queue))
        .route("/play-track-list", post(commands::play_track_list))
        .route("/play-index", post(commands::post_play_index))
        .route("/play", post(commands::post_play))
        .route("/pause", post(commands::post_pause))
        .route("/next", post(commands::post_next))
        .route("/previous", post(commands::post_previous))
        .route("/seek", post(commands::post_seek))
        .route("/reset-queue", post(commands::reset_queue))
        .route("/clear-queue", post(commands::clear_queue))
        .route("/add-to-queue", post(commands::add_to_queue))
        .route("/set-next-in-queue", post(commands::set_next_in_queue))
        .route("/remove-from-queue", post(commands::remove_from_queue))
        .route("/shuffle-queue", post(commands::shuffle_queue))
        .route("/replay-gain-mode", post(commands::replay_gain_mode))
        .route("/set-repeat", post(commands::set_repeat))
        .route("/set-shuffle", post(commands::set_shuffle))
        .route("/set-volume", post(commands::set_volume))
        .route("/set-playback-rate", post(commands::set_playback_rate))
        .layer(ServiceBuilder::new().layer(cors))
        .with_state(ctx);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;
    Ok(())
}

pub type Ctx = Arc<CtxData>;

pub struct CtxData {
    subsonic: Subsonic,
    mpd: Arc<RwLock<Mpd>>,
    events: events::MpdEvents,
}

#[derive(Debug, Error)]
#[error(transparent)]
struct AppError(#[from] anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        log::error!("{}", self.0);
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}

type AppResult<T> = Result<T, AppError>;

async fn websocket(ws: WebSocketUpgrade, ctx: State<Ctx>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run_websocket(ctx.0, socket))
}

async fn run_websocket(ctx: Ctx, socket: WebSocket) {
    let (tx, rx) = socket.split();

    let session = Session {
        ctx,
        tx: Sender::new(tx),
    };

    let receive_task = receive_task(&session, rx);
    pin_mut!(receive_task);

    let events_task = events::run_events(&session);
    pin_mut!(events_task);

    let fut = future::select(receive_task, events_task);
    let result = fut.await.factor_first().0;

    if let Err(err) = result {
        log::error!("error running websocket: {err}");
    }
}

async fn receive_task(session: &Session, rx: SplitStream<WebSocket>) -> Result<()> {
    let messages = message_stream(rx);
    pin_mut!(messages);

    while let Some(msg) = messages.next().await {
        match msg {
            ClientMsg::Command(command) => {
                commands::dispatch(session, command).await;
            }
        }
    }

    Ok(())
}

fn message_stream(rx: SplitStream<WebSocket>) -> impl Stream<Item = ClientMsg> {
    stream! {
        pin_mut!(rx);

        while let Some(msg) = rx.next().await {
            let msg = match msg {
                Ok(msg) => msg,
                Err(err) if broken_pipe(&err) => { break }
                Err(err) => {
                    log::error!("websocket receive: {err}");
                    break;
                }
            };

            let ws::Message::Text(text) = msg else { continue };
            log::debug!("rx msg: {text}");

            let msg = match serde_json::from_str(&text) {
                Ok(msg) => msg,
                Err(err) => {
                    log::warn!("json parse error in websocket message: {err}");
                    continue;
                }
            };

            yield msg;
        }
    }
}

pub struct Session {
    ctx: Ctx,
    tx: Sender,
}

impl Session {
    pub fn ctx(&self) -> Ctx {
        self.ctx.clone()
    }

    pub async fn mpd(&self) -> RwLockWriteGuard<'_, Mpd> {
        self.ctx.mpd.write().await
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct SeqNumber(pub usize);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClientMsg {
    Command(Command),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerMsg {
    Response(Response),
    Playback(events::PlaybackEvent),
    Queue(events::QueueEvent),
}

#[derive(Debug, Deserialize)]
pub struct Command {
    seq: SeqNumber,
    #[serde(flatten)]
    kind: commands::CommandKind,
}

#[derive(Debug, Serialize)]
pub struct Response {
    seq: SeqNumber,
    #[serde(flatten)]
    kind: commands::ResponseKind,
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
