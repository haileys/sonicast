use std::sync::Arc;

use crate::mpd::{self, Mpd};
use crate::subsonic::{self, Subsonic};

use anyhow::Result;
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::http::{Method, StatusCode};
use thiserror::Error;
use tokio::sync::RwLock;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};

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
        .route("/play-index", post(commands::play_index))
        .route("/play", post(commands::play))
        .route("/pause", post(commands::pause))
        .route("/next", post(commands::next))
        .route("/previous", post(commands::previous))
        .route("/seek", post(commands::seek))
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
    ws.on_upgrade(move |socket| events::run_websocket(ctx.0, socket))
}

// async fn update_status(ctx: Ctx, status_tx: watch::Sender<mpd::Status>) {
//     loop {
//         tokio::time::sleep(STATUS_INTERVAL).await;

//         let status = {
//             let mpd = ctx.mpd.read().await;
//             match mpd.status().await {
//                 Ok(status) => status,
//                 Err(err) => {
//                     log::error!("fetching mpd status: {err:?}");
//                     continue;
//                 }
//             }
//         };

//         status.state;

//         if status_tx.send(status).is_err() {
//             break;
//         }
//     }
// }
