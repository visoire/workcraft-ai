//! WorkCraft's own MCP client against the real `workcraft-cli mcp` binary: the stdio transport end to end.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use workcraft_mcp::{McpClient, McpConfig, McpHub, ServerEntry};

fn temp() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("wc-cli-{}", std::process::id()))
}

fn entry(home: &std::path::Path) -> ServerEntry {
    ServerEntry {
        command: Some(env!("CARGO_BIN_EXE_workcraft-cli").to_string()),
        args: vec!["mcp".into(), "--no-mcp".into()],
        env: BTreeMap::from([(
            "WORKCRAFT_CHAT_HOME".to_string(),
            home.display().to_string(),
        )]),
        ..ServerEntry::default()
    }
}

#[tokio::test]
async fn client_talks_to_the_real_server_over_stdio() {
    let home = temp();
    let client = McpClient::connect("workcraft", &entry(&home))
        .await
        .unwrap();
    assert_eq!(
        client.server_info().unwrap()["serverInfo"]["name"],
        "workcraft"
    );

    let tools = client.list_tools().await.unwrap();
    assert!(tools.len() >= 20, "{} tools", tools.len());
    let send = tools.iter().find(|t| t.name == "chat_send").unwrap();
    assert!(!send.read_only);
    assert!(
        tools
            .iter()
            .find(|t| t.name == "chat_list")
            .unwrap()
            .read_only
    );

    let made = client
        .call_tool("chat_new", json!({"title": "round trip"}))
        .await
        .unwrap();
    assert!(made.ok, "{}", made.text);
    let id = serde_json::from_str::<Value>(&made.text).unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let listed = client.call_tool("chat_list", json!({})).await.unwrap();
    assert!(listed.text.contains(&id));

    // A failing command is a tool error, not a transport error.
    let bad = client
        .call_tool("chat_get", json!({"session": "nope"}))
        .await
        .unwrap();
    assert!(!bad.ok);

    // The hub namespaces the same server.
    let hub = McpHub::new();
    let cfg = McpConfig {
        servers: BTreeMap::from([("wc".to_string(), entry(&home))]),
    };
    let reports = hub.connect_all(&cfg).await;
    assert!(reports[0].error.is_none(), "{:?}", reports[0].error);
    assert!(hub.tools().await.iter().any(|t| t.name == "wc__chat_send"));
    let r = hub
        .call("wc__command_list", json!({"filter": "team"}))
        .await
        .unwrap();
    assert!(r.text.contains("team.run"));
    drop(client);
    let _ = std::fs::remove_dir_all(home);
}
