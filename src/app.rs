use std::sync::Arc;

use crate::mpd::{self, Mpd};
use crate::subsonic::{self, AuthParams, Subsonic, SubsonicBase};
use crate::util::broken_pipe;

use anyhow::Result;
use async_stream::stream;
use axum::extract::State;
use axum::extract::ws::{self, WebSocket, WebSocketUpgrade};
use axum::http::Method;
use axum::response::IntoResponse;
use axum::Form;
use futures::{future, Stream};
use futures::sink::SinkExt;
use futures::stream::{SplitSink, SplitStream};
use futures::{pin_mut, StreamExt};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, RwLockWriteGuard, Mutex as AsyncMutex};
use tower_http::cors::{Any, CorsLayer};
use tower::ServiceBuilder;

mod commands;
mod events;
mod helper;
mod types;

pub struct Config {
    pub listen: String,
    pub subsonic: subsonic::Config,
    pub mpd: mpd::Config,
}

pub async fn run(config: &Config) -> Result<()> {
    use axum::Router;
    use axum::routing::get;

    // open clients, including two mpd connections
    //  - one for commands, the other for events
    let subsonic = SubsonicBase::new(&config.subsonic);
    let mpd = Mpd::connect(&config.mpd).await?;
    let mpd_event = Mpd::connect(&config.mpd).await?;

    let mpd = Arc::new(RwLock::new(mpd));
    let ctx = Ctx::new(AppData {
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
        .layer(ServiceBuilder::new().layer(cors))
        .with_state(ctx);

    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

pub type Ctx = Arc<AppData>;

pub struct AppData {
    subsonic: SubsonicBase,
    mpd: Arc<RwLock<Mpd>>,
    events: events::MpdEvents,
}

async fn websocket(
    ctx: State<Ctx>,
    ws: WebSocketUpgrade,
    auth: Form<AuthParams>,
) -> Result<impl IntoResponse, StatusCode> {
    let subsonic = ctx.subsonic.authenticate(auth.0).await
        .map_err(|err| {
            log::warn!("subsonic authenticate: {err:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(ws.on_upgrade(move |socket| {
        run_websocket(ctx.0, socket, subsonic)
    }))
}

async fn run_websocket(ctx: Ctx, socket: WebSocket, subsonic: Subsonic) {
    let (tx, rx) = socket.split();

    let session = Session {
        ctx,
        tx: Sender::new(tx),
        subsonic,
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
    subsonic: Subsonic,
}

impl Session {
    pub async fn mpd(&self) -> RwLockWriteGuard<'_, Mpd> {
        self.ctx.mpd.write().await
    }

    pub fn resolver(&self) -> helper::Resolver {
        helper::Resolver::new(&self.subsonic)
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
