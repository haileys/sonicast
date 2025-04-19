use std::str::FromStr;

use anyhow::{Context, anyhow, bail};
use thiserror::Error;
use tokio::io::{BufReader, AsyncRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub struct MpdReader {
    r: BufReader<Box<dyn AsyncRead + Sync + Send + Unpin>>,
}

pub struct Protocol {
    pub version: String,
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("io error")]
    Io(#[from] std::io::Error),
    #[error("protocol error")]
    ProtocolError(#[from] anyhow::Error),
}

impl MpdReader {
    pub async fn open<R>(r: R) -> anyhow::Result<(Self, Protocol)>
        where R: AsyncRead + Sync + Send + Unpin + 'static
    {
        let mut r = BufReader::new(Box::new(r) as Box<_>);

        let mut line = String::new();
        r.read_line(&mut line).await?;
        let line = line.trim_end();

        let Some(proto) = prefixed("OK MPD ", &line) else {
            bail!("unexpected initial line from mpd: {line:?}")
        };

        let reader = MpdReader { r };
        let protocol = Protocol { version: proto.to_string() };

        Ok((reader, protocol))
    }

    pub async fn read_response(&mut self) -> Result<Response, Error> {
        let mut attributes = Attributes::default();
        let mut binary = None;

        let mut buff = String::new();
        loop {
            buff.truncate(0);
            self.r.read_line(&mut buff).await?;
            if buff.len() == 0 {
                return Err(Error::ProtocolError(anyhow!("connection eof")));
            }

            let line = buff.trim_end();
            log::trace!("recv: {line}");

            if line == "OK" {
                return Ok(Ok(OkResponse {
                    attributes,
                    binary,
                }));
            }

            if let Some(line) = prefixed("ACK ", line) {
                let line = line.to_string();
                return Ok(Err(ErrorResponse { line }));
            }

            if let Some(len) = prefixed("binary: ", line) {
                binary = Some(self.read_binary(len).await?);
                continue;
            }

            if let Some((key, value)) = line.split_once(":") {
                let value = value.trim_start();
                attributes.attrs.push((key.to_string(), value.to_string()));
            } else {
                return Err(Error::ProtocolError(anyhow!("unrecognised response line from mpd: {line:?}")));
            }
        }
    }

    async fn read_binary(&mut self, len: &str) -> anyhow::Result<Vec<u8>> {
        let len = len.parse().context("parsing length of binary data")?;
        let mut bin = Vec::with_capacity(len);
        self.r.read_exact(&mut bin).await.context("reading binary data")?;
        let nl = self.r.read_u8().await.context("reading binary trailing newline")?;
        if nl != b'\n' {
            bail!("binary data did not end with trailing newline");
        }
        Ok(bin)
    }
}

fn prefixed<'a>(prefix: &str, s: &'a str) -> Option<&'a str> {
    if s.starts_with(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

pub type Response = Result<OkResponse, ErrorResponse>;

#[derive(Error, Debug)]
#[error("command returned error: {line}")]
pub struct ErrorResponse {
    pub line: String,
}

#[derive(Debug)]
pub struct OkResponse {
    pub attributes: Attributes,
    pub binary: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
pub struct Attributes {
    attrs: Vec<(String, String)>
}

impl Attributes {
    pub fn get<T: FromStr<Err = E>, E: Send + Sync + std::error::Error + 'static>(&self, name: &str) -> anyhow::Result<T> {
        Ok(self.get_one(name)
            .ok_or_else(|| anyhow!("missing {name} attribute"))?
            .parse()
            .with_context(|| format!("malformed {name} attribute"))?)
    }

    pub fn get_opt<T: FromStr<Err = E>, E: Send + Sync + std::error::Error + 'static>(&self, name: &str) -> anyhow::Result<Option<T>> {
        self.get_one(name)
            .map(|value| value.parse().with_context(|| format!("malformed {name} attribute")))
            .transpose()
    }

    pub fn get_one(&self, name: &str) -> Option<&'_ str> {
        Some(&self.attrs.iter().find(|(k, _)| k == name)?.1)
    }

    pub fn get_all<'a, 'n: 'a>(&'a self, name: &'n str) -> impl Iterator<Item = &'a str> {
        self.attrs.iter().filter_map(move |(k, v)| {
            if k == name {
                Some(v.as_str())
            } else {
                None
            }
        })
    }

    pub fn split_at(self, name: &str) -> Vec<Attributes> {
        let mut splits = Vec::new();

        for (k, v) in self.attrs {
            if k == name {
                splits.push(Attributes::default());
            }

            if let Some(split) = splits.last_mut() {
                split.attrs.push((k, v));
            }
        }

        splits
    }

    #[allow(unused)]
    pub fn iter(&self) -> impl Iterator<Item = (&'_ str, &'_ str)> {
        self.attrs.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

pub struct MpdWriter {
    w: Box<dyn AsyncWrite + Send + Sync + Unpin>,
}

impl MpdWriter {
    pub fn open<W>(w: W) -> Self
        where W: AsyncWrite + Send + Sync + Unpin + 'static
    {
        MpdWriter { w: Box::new(w) }
    }

    pub async fn send_command(&mut self, cmd: &str, args: &[&str]) -> anyhow::Result<()> {
        let mut line = cmd.to_string();
        for arg in args {
            line.push(' ');
            line.push('"');
            for c in arg.chars() {
                match c {
                    '"' | '\\' => {
                        line.push('\\');
                        line.push(c);
                    }
                    '\n' => {
                        bail!("newline in command argument");
                    }
                    _ => {
                        line.push(c);
                    }
                }
            }
            line.push('"');
        }
        line.push('\n');

        self.w.write_all(line.as_bytes()).await?;
        log::trace!("send: {}", line.trim());
        Ok(())
    }
}
