//! Control channel client. Used to drive a running WorkCraft and the Craft apps' `--control` ports,
//! which speak the same line protocol.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use workcraft_core::{Error, Result};

pub struct ControlClient {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    next: u64,
}

impl ControlClient {
    /// Connect to a loopback address and authenticate. Anything but loopback is refused.
    pub async fn connect(addr: &str, token: &str) -> Result<Self> {
        let sock: std::net::SocketAddr = addr
            .parse()
            .map_err(|_| Error::Invalid(format!("bad address '{addr}'")))?;
        if !sock.ip().is_loopback() {
            return Err(Error::Denied("the control channel is loopback only".into()));
        }
        let stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(sock))
            .await
            .map_err(|_| Error::Timeout(format!("connect {addr}")))?
            .map_err(|e| Error::Io(format!("connect {addr}: {e}")))?;
        let (r, w) = stream.into_split();
        let mut c = Self {
            reader: BufReader::new(r),
            writer: w,
            next: 1,
        };
        let r = c
            .call_raw(
                Value::String("auth".into()),
                "auth",
                json!({ "token": token }),
            )
            .await?;
        match r.get("error") {
            Some(e) => Err(Error::Denied(
                e.as_str().unwrap_or("auth failed").to_string(),
            )),
            None => Ok(c),
        }
    }

    /// Send a request and wait for the reply with the same id. Lines without that id (events) are skipped.
    async fn call_raw(&mut self, id: Value, method: &str, params: Value) -> Result<Value> {
        let mut line = json!({ "id": id, "method": method, "params": params }).to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = tokio::time::timeout(Duration::from_secs(120), self.reader.read_line(&mut buf))
                .await
                .map_err(|_| Error::Timeout(method.to_string()))??;
            if n == 0 {
                return Err(Error::Io("control channel closed".into()));
            }
            if let Ok(v) = serde_json::from_str::<Value>(buf.trim())
                && v.get("id") == Some(&id)
            {
                return Ok(v);
            }
        }
    }

    /// Call a method; an `error` reply becomes `Err`.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = json!(self.next);
        self.next += 1;
        let v = self.call_raw(id, method, params).await?;
        if let Some(e) = v.get("error") {
            let msg = e.as_str().map_or_else(|| e.to_string(), str::to_string);
            return Err(Error::Tool(msg));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }

    /// `engine.execute`: run one command id with params.
    pub async fn execute(&mut self, command: &str, params: Value) -> Result<Value> {
        self.call(
            "engine.execute",
            json!({ "command": command, "params": params }),
        )
        .await
    }

    pub async fn commands(&mut self) -> Result<Value> {
        self.call("engine.commands", Value::Null).await
    }
}
