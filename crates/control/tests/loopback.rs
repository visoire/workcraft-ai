use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use workcraft_control::client::ControlClient;
use workcraft_control::server::ControlServer;
use workcraft_control::{CommandBus, CommandInfo, new_token};
use workcraft_core::{Error, Event, Result};

struct Bus {
    events: broadcast::Sender<Event>,
}

#[async_trait]
impl CommandBus for Bus {
    fn commands(&self) -> Vec<CommandInfo> {
        vec![CommandInfo {
            id: "test.echo".into(),
            title: "Echo".into(),
            description: "Return the params".into(),
            params: json!({"type":"object"}),
        }]
    }

    async fn execute(&self, id: &str, params: Value) -> Result<Value> {
        match id {
            "test.echo" => Ok(params),
            "test.event" => {
                let _ = self.events.send(Event::TurnStarted {
                    session: "s1".into(),
                });
                Ok(json!({}))
            }
            other => Err(Error::NotFound(format!("command {other}"))),
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }
}

async fn server() -> ControlServer {
    let (events, _) = broadcast::channel(16);
    let bus: Arc<dyn CommandBus> = Arc::new(Bus { events });
    ControlServer::start(bus, 0, new_token()).await.unwrap()
}

#[tokio::test]
async fn auth_execute_and_list() {
    let s = server().await;
    let mut c = ControlClient::connect(&s.addr.to_string(), &s.token)
        .await
        .unwrap();
    let r = c.execute("test.echo", json!({"a": 1})).await.unwrap();
    assert_eq!(r["a"], 1);
    let cmds = c.commands().await.unwrap();
    assert_eq!(cmds[0]["id"], "test.echo");
    assert!(c.execute("nope", Value::Null).await.is_err());
    assert!(c.execute("", Value::Null).await.is_err());
}

#[tokio::test]
async fn wrong_token_is_refused() {
    let s = server().await;
    let bad = ControlClient::connect(&s.addr.to_string(), &new_token()).await;
    assert!(matches!(bad, Err(Error::Denied(_))));
}

#[tokio::test]
async fn non_loopback_is_refused_by_the_client() {
    let r = ControlClient::connect("192.168.1.5:7878", "x").await;
    assert!(matches!(r, Err(Error::Denied(_))));
}

#[tokio::test]
async fn requests_before_auth_are_refused() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let s = server().await;
    let stream = tokio::net::TcpStream::connect(s.addr).await.unwrap();
    let (r, mut w) = stream.into_split();
    w.write_all(b"{\"id\":1,\"method\":\"engine.commands\"}\n")
        .await
        .unwrap();
    let mut line = String::new();
    BufReader::new(r).read_line(&mut line).await.unwrap();
    assert!(line.contains("unauthorized"));
}

#[tokio::test]
async fn events_are_streamed_to_subscribers() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let s = server().await;
    let stream = tokio::net::TcpStream::connect(s.addr).await.unwrap();
    let (r, mut w) = stream.into_split();
    let mut r = BufReader::new(r);
    let auth = format!(
        "{{\"id\":\"auth\",\"method\":\"auth\",\"params\":{{\"token\":\"{}\"}}}}\n",
        s.token
    );
    w.write_all(auth.as_bytes()).await.unwrap();
    w.write_all(b"{\"id\":2,\"method\":\"events.subscribe\"}\n")
        .await
        .unwrap();
    let mut line = String::new();
    r.read_line(&mut line).await.unwrap();
    line.clear();
    r.read_line(&mut line).await.unwrap();
    assert!(line.contains("subscribed"));
    w.write_all(
        b"{\"id\":3,\"method\":\"engine.execute\",\"params\":{\"command\":\"test.event\"}}\n",
    )
    .await
    .unwrap();
    let mut saw_event = false;
    for _ in 0..4 {
        line.clear();
        r.read_line(&mut line).await.unwrap();
        if line.contains("\"event\"") && line.contains("turn_started") {
            saw_event = true;
            break;
        }
    }
    assert!(saw_event);
}
