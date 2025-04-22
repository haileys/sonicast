pub mod protocol;
pub mod types;

use std::borrow::Cow;
use std::cmp;
use std::collections::VecDeque;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use derive_more::Display;
use protocol::OkResponse;
use tokio::net::UnixStream;
use tokio::sync::{oneshot, Mutex as AsyncMutex};

use protocol::{MpdReader, MpdWriter, Protocol, Response, Attributes};
use types::{Changed, Id, Playlist, PlaylistItem, ReplayGainMode, Status};

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(1);

pub struct Mpd {
    conn: Conn,
}

pub struct Config {
    pub socket: PathBuf,
}

impl Mpd {
    pub async fn connect(config: &Config) -> Result<Mpd> {
        let (conn, proto) = Conn::connect(config).await?;
        log::info!("Connected to mpd at {}, protocol version {}",
            config.socket.display(), proto.version);
        Ok(Mpd { conn })
    }

    pub async fn addid(&self, location: &str) -> Result<Id> {
        let resp = self.conn.command("addid", &[location]).await?;
        resp.attributes.get("Id")
    }

    pub async fn delete(&self, pos: isize) -> Result<()> {
        let pos = position(pos);
        self.conn.command("deleteid", &[&pos]).await?;
        Ok(())
    }

    #[allow(unused)]
    pub async fn deleteid(&self, id: &Id) -> Result<()> {
        self.conn.command("deleteid", &[id.as_str()]).await?;
        Ok(())
    }

    pub async fn clear(&self) -> Result<()> {
        self.conn.command("clear", &[]).await?;
        Ok(())
    }

    pub async fn playlistinfo(&self) -> Result<Playlist> {
        let resp = self.conn.command("playlistinfo", &[]).await?;

        let items = resp.attributes.split_at("file")
            .into_iter()
            .map(parse_playlist_item)
            .collect::<Result<Vec<_>>>()
            .context("parsing playlist info response")?;

        Ok(Playlist { items })
    }

    pub async fn playlistclear(&self, name: &str) -> Result<()> {
        self.conn.command("playlistclear", &[name]).await?;
        Ok(())
    }

    pub async fn playlistadd(&self, name: &str, location: &str) -> Result<()> {
        self.conn.command("playlistadd", &[name, location]).await?;
        Ok(())
    }

    pub async fn load(&self, name: &str, range: Option<Range<usize>>, pos: Option<isize>) -> Result<()> {
        let range = match range {
            None => Cow::Borrowed("0:"),
            Some(range) => Cow::Owned(format!("{}:{}", range.start, range.end)),
        };

        let pos = match pos {
            None => Cow::Borrowed(""),
            Some(pos) => Cow::Owned(position(pos)),
        };

        self.conn.command("load", &[name, &range, &pos]).await?;
        Ok(())
    }

    pub async fn idle(&self) -> Result<Changed> {
        const SUBSYSTEMS: &[&str] = &[
            "player",
            "playlist",
            "options",
            "mixer",
        ];
        let resp = self.conn.command("idle", SUBSYSTEMS).await?;
        Ok(Changed::from_attributes(&resp.attributes)?)
    }

    pub async fn play(&self) -> Result<()> {
        self.conn.command("play", &[]).await?;
        Ok(())
    }

    pub async fn playpos(&self, pos: usize) -> Result<()> {
        let pos = pos.to_string();
        self.conn.command("play", &[&pos]).await?;
        Ok(())
    }

    #[allow(unused)]
    pub async fn playid(&self, id: Id) -> Result<()> {
        self.conn.command("playid", &[id.as_str()]).await?;
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        self.conn.command("stop", &[]).await?;
        Ok(())
    }

    pub async fn pause(&self) -> Result<()> {
        self.conn.command("pause", &[]).await?;
        Ok(())
    }

    pub async fn next(&self) -> Result<()> {
        self.conn.command("next", &[]).await?;
        Ok(())
    }

    pub async fn previous(&self) -> Result<()> {
        self.conn.command("previous", &[]).await?;
        Ok(())
    }

    pub async fn seek(&self, index: usize, time: f64) -> Result<()> {
        let index = format!("{index}");
        let time = format!("{time}");
        self.conn.command("seek", &[&index, &time]).await?;
        Ok(())
    }

    pub async fn seekcur(&self, pos: f64) -> Result<()> {
        let pos = format!("{pos}");
        self.conn.command("seekcur", &[&pos]).await?;
        Ok(())
    }

    pub async fn status(&self) -> Result<Status> {
        let resp = self.conn.command("status", &[]).await?;
        Ok(Status::from_attributes(&resp.attributes)?)
    }

    pub async fn replay_gain_status(&self) -> Result<ReplayGainMode> {
        let resp = self.conn.command("replay_gain_status", &[]).await?;
        let mode = resp.attributes.get_opt("replay_gain_mode")?;
        Ok(mode.unwrap_or(ReplayGainMode::None))
    }

    #[allow(unused)]
    pub async fn playlistid(&self, id: &Id) -> Result<PlaylistItem> {
        let resp = self.conn.command("playlistid", &[id.as_str()]).await?;
        parse_playlist_item(resp.attributes)
    }

    pub async fn random(&self, shuffle: bool) -> Result<()> {
        self.conn.command("random", &[boolean(shuffle)]).await?;
        Ok(())
    }

    pub async fn repeat(&self, repeat: bool) -> Result<()> {
        self.conn.command("repeat", &[boolean(repeat)]).await?;
        Ok(())
    }

    pub async fn shuffle(&self) -> Result<()> {
        self.conn.command("shuffle", &[]).await?;
        Ok(())
    }

    pub async fn setvol(&self, volume: usize) -> Result<()> {
        let volume = cmp::min(100, volume);
        let volume = volume.to_string();
        self.conn.command("setvol", &[&volume]).await?;
        Ok(())
    }

    pub async fn replay_gain_mode(&self, mode: ReplayGainMode) -> Result<()> {
        let mode = match mode {
            ReplayGainMode::None => "none",
            ReplayGainMode::Track => "track",
            ReplayGainMode::Album => "album",
            ReplayGainMode::Auto => "auto",
        };

        self.conn.command("replay_gain_mode", &[mode]).await?;
        Ok(())
    }
}

fn position(pos: isize) -> String {
    format!("{pos:+}")
}

fn boolean(b: bool) -> &'static str {
    if b { "1" } else { "0" }
}

fn parse_playlist_item(attrs: Attributes) -> Result<PlaylistItem> {
    Ok(PlaylistItem {
        file: attrs.get("file")?,
        pos: attrs.get("Pos")?,
        id: attrs.get("Id")?,
        title: attrs.get_one("Title").map(str::to_owned),
        name: attrs.get_one("Name").map(str::to_owned),
    })
}

struct Conn {
    reader: tokio::task::JoinHandle<()>,
    keepalive: tokio::task::JoinHandle<()>,
    shared: Arc<ConnShared>,
}

struct ConnShared {
    writer: AsyncMutex<MpdWriter>,
    queue: ResponseQueue,
}

type ResponseQueue = Arc<AsyncMutex<VecDeque<ResponseWait>>>;

struct ResponseWait {
    finish: oneshot::Sender<Response>,
}

#[derive(Debug, Display)]
#[display("args: {args:?}")]
struct Command {
    command: String,
    args: Vec<String>,
}

impl Conn {
    pub async fn connect(config: &Config) -> Result<(Conn, Protocol)> {
        let sock = UnixStream::connect(&config.socket).await?;
        let (rx, tx) = sock.into_split();
        let (reader, proto) = MpdReader::open(rx).await?;

        let shared = Arc::new(ConnShared {
            writer: tokio::sync::Mutex::new(MpdWriter::open(tx)),
            queue: ResponseQueue::default(),
        });

        let reader = tokio::task::spawn(conn_reader(reader, shared.clone()));
        let keepalive = tokio::task::spawn(conn_keepalive(shared.clone()));

        Ok((Conn { reader, keepalive, shared }, proto))
    }

    async fn command(&self, cmd: &str, args: &[&str]) -> Result<OkResponse> {
        let result = try_command(&self.shared, cmd, args).await;

        ok_response(result).with_context(|| Command {
            command: cmd.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
        })
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.reader.abort();
        self.keepalive.abort();
    }
}

fn ok_response(result: Result<Response>) -> Result<OkResponse> {
    Ok(result??)
}

async fn try_command(shared: &ConnShared, cmd: &str, args: &[&str]) -> Result<Response> {
    let (tx, rx) = oneshot::channel();

    // first take async lock on writer to write command to socket
    let mut writer = shared.writer.lock().await;

    // take sync lock on queue while still holding async writer
    // lock to ensure correct queue ordering
    {
        let mut queue = shared.queue.lock().await;
        queue.push_back(ResponseWait { finish: tx });

        writer.send_command(cmd, args).await?;
    }

    // if command is "idle", then the connection goes into a special state
    // where it is invalid to send additional pipelined commands. the simple
    // way to enforce this is to continue holding the async writer lock while
    // waiting for a response. for all other commands, unlock here.
    if !is_idle(cmd) {
        drop(writer);
    }

    Ok(rx.await?)
}

fn is_idle(cmd: &str) -> bool {
    cmd.trim_ascii().eq_ignore_ascii_case("idle")
}

async fn conn_reader(mut reader: MpdReader, shared: Arc<ConnShared>) {
    loop {
        let response = reader.read_response().await
            .expect("lost mpd connection");

        let mut queue = shared.queue.lock().await;
        let Some(front) = queue.pop_front() else { unreachable!() };
        let _ = front.finish.send(response);
    }
}

async fn conn_keepalive(shared: Arc<ConnShared>) {
    loop {
        tokio::time::sleep(KEEPALIVE_INTERVAL).await;

        match try_command(&shared, "ping", &[]).await {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => {
                log::warn!("error pinging in keepalive task: {err}");
            }
            Err(_) => break
        }
    }
}
