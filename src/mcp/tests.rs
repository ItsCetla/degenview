//! End-to-end tests over a real loopback socket, with a stub standing in for the app thread.

use crate::mcp::McpServer;
use crate::mcp::auth::verifier_hex;
use crate::mcp::bridge::{self, McpCommand, McpError, McpReply};
use crate::mcp::state::McpShared;
use crate::mcp::types::{AddChartResult, ChartsResult, WorkspaceInfo};
use crate::model::InstrumentId;
use crate::model::mcp::{McpClient, Permission, PermissionSet};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub struct Harness {
    pub shared: Arc<McpShared>,
    pub port: u16,
    pub added: Arc<Mutex<Vec<(u64, InstrumentId)>>>,
    server: Option<McpServer>,
    stop_app: Arc<AtomicBool>,
    app: Option<JoinHandle<()>>,
}

fn client(id: &str, token: &str, permissions: &[Permission]) -> McpClient {
    McpClient {
        id: id.into(),
        name: format!("client {id}"),
        verifier: verifier_hex(token),
        permissions: PermissionSet::of(permissions.iter().copied()),
        created_at_ms: 0,
        last_used_ms: None,
    }
}

impl Harness {
    /// `clients`: `(id, token, grants)`. The global cap allows everything unless changed later.
    pub fn start(clients: &[(&str, &str, &[Permission])]) -> Self {
        let shared = McpShared::new();
        shared.replace(
            clients
                .iter()
                .map(|(id, token, perms)| client(id, token, perms))
                .collect(),
            PermissionSet::of(Permission::ALL),
        );
        let (bridge, inbox) = bridge::channel(shared.clone(), Arc::new(|| {}));
        let server = McpServer::start(shared.clone(), bridge, 0).expect("server starts");
        let port = match shared.status() {
            crate::mcp::state::McpStatus::Running { port } => port,
            other => panic!("not running: {other:?}"),
        };
        let added = Arc::new(Mutex::new(Vec::new()));
        let stop_app = Arc::new(AtomicBool::new(false));
        let app = {
            let shared = shared.clone();
            let added = added.clone();
            let stop = stop_app.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    for request in inbox.take(8) {
                        let added = added.clone();
                        request.run(&shared, move |command| stub_app(command, &added));
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            })
        };
        Self {
            shared,
            port,
            added,
            server: Some(server),
            stop_app,
            app: Some(app),
        }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    pub fn stop_server(&mut self) {
        if let Some(mut server) = self.server.take() {
            server.stop();
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.stop_server();
        self.stop_app.store(true, Ordering::Release);
        if let Some(app) = self.app.take() {
            let _ = app.join();
        }
    }
}

fn stub_app(
    command: McpCommand,
    added: &Mutex<Vec<(u64, InstrumentId)>>,
) -> Result<McpReply, McpError> {
    match command {
        McpCommand::ListWorkspaces => Ok(McpReply::Workspaces(vec![WorkspaceInfo {
            workspace_id: "1".into(),
            title: "Unnamed".into(),
            chart_count: 0,
            active: true,
            timeframe: "1D".into(),
        }])),
        McpCommand::ListCharts { workspace_id: 1 } => Ok(McpReply::Charts(ChartsResult {
            workspace_id: "1".into(),
            charts: Vec::new(),
        })),
        McpCommand::ListCharts { .. } => Err(McpError::NotFound("No such workspace".into())),
        McpCommand::AddChart {
            workspace_id,
            instrument,
        } => {
            if workspace_id != 1 {
                return Err(McpError::NotFound("No such workspace".into()));
            }
            added
                .lock()
                .unwrap()
                .push((workspace_id, instrument.clone()));
            Ok(McpReply::ChartAdded(AddChartResult {
                workspace_id: "1".into(),
                chart_id: "2".into(),
                instrument: instrument.key(),
                source: "binance".into(),
                symbol: instrument.symbol.clone(),
                display_symbol: instrument.symbol.clone(),
                timeframe: "1D".into(),
                status: "created".into(),
            }))
        }
    }
}

pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| {
            // An SSE-framed answer carries the JSON on its `data:` line.
            self.body
                .lines()
                .find_map(|line| line.strip_prefix("data:"))
                .and_then(|data| serde_json::from_str(data.trim()).ok())
                .unwrap_or(Value::Null)
        })
    }
}

pub fn post(url: &str, token: Option<&str>, headers: &[(&str, &str)], body: &str) -> Reply {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let mut request = agent
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");
    if let Some(token) = token {
        request = request.header("Authorization", &format!("Bearer {token}"));
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    match request.send(body) {
        Ok(mut response) => Reply {
            status: response.status().as_u16(),
            body: response.body_mut().read_to_string().unwrap_or_default(),
        },
        Err(error) => Reply {
            status: 0,
            body: error.to_string(),
        },
    }
}

const READ: &[Permission] = &[Permission::ChartsRead];
const BOTH: &[Permission] = &[Permission::ChartsRead, Permission::ChartsCreate];
const PROTOCOL: (&str, &str) = ("MCP-Protocol-Version", "2025-06-18");

fn rpc(url: &str, token: Option<&str>, method: &str, params: Value) -> Reply {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    post(url, token, &[PROTOCOL], &body.to_string())
}

fn call(url: &str, token: &str, name: &str, arguments: Value) -> Value {
    let reply = rpc(
        url,
        Some(token),
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply.json()["result"].clone()
}

fn tool_names(url: &str, token: &str) -> Vec<String> {
    let reply = rpc(url, Some(token), "tools/list", json!({}));
    assert_eq!(reply.status, 200, "{}", reply.body);
    let mut names: Vec<String> = reply.json()["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    names
}

fn error_code(result: &Value) -> String {
    assert_eq!(result["isError"], json!(true), "{result}");
    result["structuredContent"]["error"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn raw_request(port: u16, head: &str, body: &str) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let request = format!(
        "POST /mcp HTTP/1.1\r\n{head}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response
}

fn status_line(response: &str) -> &str {
    response.lines().next().unwrap_or_default()
}

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;

#[test]
fn every_request_needs_a_valid_token() {
    let harness = Harness::start(&[("c1", "good-token", BOTH)]);
    let url = harness.url();
    let init = json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}});

    for token in [
        None,
        Some(""),
        Some("wrong"),
        Some("good-toke"),
        Some("good-token2"),
    ] {
        let reply = rpc(&url, token, "initialize", init.clone());
        assert_eq!(reply.status, 401, "token {token:?}");
        assert!(!reply.body.contains("good-token"));
    }
    // Wrong scheme, and a token in the URL instead of the header, are not accepted.
    let body = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":init}).to_string();
    let basic = post(&url, None, &[("Authorization", "Basic good-token")], &body);
    assert_eq!(basic.status, 401);
    let query = post(
        &format!("{url}?token=good-token&access_token=good-token"),
        None,
        &[],
        &body,
    );
    assert_eq!(query.status, 401);
    // Not even an unknown path answers without a token.
    let other = post(
        &format!("http://127.0.0.1:{}/other", harness.port),
        None,
        &[],
        &body,
    );
    assert_eq!(other.status, 401);
    let reply = post(
        &format!("http://127.0.0.1:{}/other", harness.port),
        Some("good-token"),
        &[],
        &body,
    );
    assert_eq!(reply.status, 404);

    assert_eq!(
        rpc(&url, Some("good-token"), "initialize", init).status,
        200
    );
}

#[test]
fn discovery_and_calls_work_for_a_permitted_client() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    assert_eq!(
        tool_names(&url, "tok"),
        [
            "degenview_add_chart",
            "degenview_list_charts",
            "degenview_list_workspaces"
        ]
    );

    let workspaces = call(&url, "tok", "degenview_list_workspaces", json!({}));
    assert_eq!(workspaces["isError"], json!(false));
    assert_eq!(
        workspaces["structuredContent"]["workspaces"][0]["workspace_id"],
        "1"
    );

    let charts = call(
        &url,
        "tok",
        "degenview_list_charts",
        json!({"workspace_id": "1"}),
    );
    assert_eq!(charts["structuredContent"]["workspace_id"], "1");
    let missing = call(
        &url,
        "tok",
        "degenview_list_charts",
        json!({"workspace_id": "99"}),
    );
    assert_eq!(error_code(&missing), "NOT_FOUND");

    let added = call(
        &url,
        "tok",
        "degenview_add_chart",
        json!({"workspace_id": "1", "source": "binance", "symbol": " btcusdt "}),
    );
    assert_eq!(added["isError"], json!(false), "{added}");
    assert_eq!(added["structuredContent"]["chart_id"], "2");
    // The app side received the validated, normalised market.
    let seen = harness.added.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, 1);
    assert_eq!(seen[0].1.key(), "BINANCE:BTCUSDT");
}

#[test]
fn bad_arguments_never_reach_the_app() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    let cases = [
        json!({"workspace_id": "1", "source": "binance", "symbol": "BTC-USD"}),
        json!({"workspace_id": "1", "source": "binance", "symbol": ""}),
        json!({"workspace_id": "1", "source": "binance", "symbol": "BTCUSDT|title"}),
        json!({"workspace_id": "one", "source": "binance", "symbol": "BTCUSDT"}),
        json!({"workspace_id": "-1", "source": "binance", "symbol": "BTCUSDT"}),
        json!({"workspace_id": "1", "source": "polymarket", "symbol": "0xabc"}),
        json!({"workspace_id": "1", "source": "binance", "symbol": "BTCUSDT", "timeframe": "1h"}),
        json!({"workspace_id": "1", "symbol": "BTCUSDT"}),
        json!({"workspace_id": "1", "source": "binance", "symbol": "B".repeat(500)}),
    ];
    for arguments in cases {
        let reply = rpc(
            &url,
            Some("tok"),
            "tools/call",
            json!({"name": "degenview_add_chart", "arguments": arguments}),
        );
        let json = reply.json();
        let rejected = json.get("error").is_some() || json["result"]["isError"] == json!(true);
        assert!(rejected, "accepted {arguments}: {}", reply.body);
    }
    assert!(harness.added.lock().unwrap().is_empty());
}

#[test]
fn permissions_are_enforced_on_every_call_not_just_discovery() {
    let harness = Harness::start(&[
        ("reader", "read-tok", READ),
        ("writer", "write-tok", BOTH),
        ("none", "none-tok", &[]),
    ]);
    let url = harness.url();

    assert_eq!(
        tool_names(&url, "read-tok"),
        ["degenview_list_charts", "degenview_list_workspaces"]
    );
    assert!(tool_names(&url, "none-tok").is_empty());

    // Guessing the name of a hidden tool is denied, and nothing runs.
    let add = json!({"workspace_id": "1", "source": "binance", "symbol": "BTCUSDT"});
    let denied = call(&url, "read-tok", "degenview_add_chart", add.clone());
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
    let denied = call(&url, "none-tok", "degenview_list_workspaces", json!({}));
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
    assert!(harness.added.lock().unwrap().is_empty());

    // Two clients with different rights, at the same time.
    let handles: Vec<_> = [
        ("read-tok", false),
        ("write-tok", true),
        ("read-tok", false),
        ("write-tok", true),
    ]
    .into_iter()
    .map(|(token, may_add)| {
        let url = url.clone();
        let add = add.clone();
        std::thread::spawn(move || {
            let result = call(&url, token, "degenview_add_chart", add);
            (may_add, result["isError"] == json!(false))
        })
    })
    .collect();
    for handle in handles {
        let (may_add, added) = handle.join().unwrap();
        assert_eq!(may_add, added);
    }

    // The global cap overrides a client's own grant, immediately.
    harness.shared.replace(
        harness.shared.clients(),
        PermissionSet::of([Permission::ChartsRead]),
    );
    let denied = call(&url, "write-tok", "degenview_add_chart", add.clone());
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
    assert_eq!(
        tool_names(&url, "write-tok"),
        ["degenview_list_charts", "degenview_list_workspaces"]
    );
}

#[test]
fn revoking_a_credential_takes_effect_on_the_next_request() {
    let harness = Harness::start(&[("c1", "tok", BOTH), ("c2", "other", BOTH)]);
    let url = harness.url();
    assert_eq!(rpc(&url, Some("tok"), "tools/list", json!({})).status, 200);

    let remaining: Vec<_> = harness
        .shared
        .clients()
        .into_iter()
        .filter(|c| c.id != "c1")
        .collect();
    harness.shared.replace(remaining, harness.shared.global());
    assert_eq!(rpc(&url, Some("tok"), "tools/list", json!({})).status, 401);
    assert_eq!(
        rpc(&url, Some("other"), "tools/list", json!({})).status,
        200
    );
}

#[test]
fn foreign_hosts_and_browser_origins_are_rejected() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let port = harness.port;
    let auth = "Authorization: Bearer tok\r\n";
    let good_host = format!("Host: 127.0.0.1:{port}\r\n");

    let ok = raw_request(port, &format!("{auth}{good_host}"), INITIALIZE);
    assert!(status_line(&ok).contains("200"), "{ok}");
    let localhost = raw_request(
        port,
        &format!("{auth}Host: localhost:{port}\r\n"),
        INITIALIZE,
    );
    assert!(status_line(&localhost).contains("200"), "{localhost}");

    // DNS rebinding: the attacker's name resolves to 127.0.0.1 but the Host header gives it away.
    for host in [
        "Host: evil.example\r\n".to_owned(),
        format!("Host: evil.example:{port}\r\n"),
        "Host: 127.0.0.1:1\r\n".to_owned(),
    ] {
        let response = raw_request(port, &format!("{auth}{host}"), INITIALIZE);
        assert!(
            !status_line(&response).contains("200"),
            "{host}: {response}"
        );
    }
    // A request from a web page carries an Origin; a native MCP client does not.
    for origin in [
        "https://evil.example",
        "http://127.0.0.1",
        &format!("http://localhost:{port}"),
        "null",
    ] {
        let response = raw_request(
            port,
            &format!("{auth}{good_host}Origin: {origin}\r\n"),
            INITIALIZE,
        );
        assert!(
            !status_line(&response).contains("200"),
            "{origin}: {response}"
        );
    }
}

#[test]
fn oversized_bodies_are_refused() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let padding = "x".repeat(crate::mcp::transport::MAX_BODY_BYTES + 1024);
    let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"degenview_list_workspaces","arguments":{"pad": padding}}});
    let reply = post(&harness.url(), Some("tok"), &[PROTOCOL], &body.to_string());
    assert_eq!(reply.status, 413, "{}", reply.body);
    assert_eq!(
        rpc(&harness.url(), Some("tok"), "tools/list", json!({})).status,
        200
    );
}

#[test]
fn listens_on_loopback_only() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    // A wildcard bind on the same port would collide only if ours were bound to a wildcard.
    let wildcard = std::net::TcpListener::bind(("0.0.0.0", harness.port));
    assert!(
        wildcard.is_ok(),
        "the server must not hold the wildcard address"
    );
    drop(wildcard);
    // And a non-loopback address of this machine is not served.
    let lan = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| socket.connect("192.0.2.1:9").map(|()| socket))
        .and_then(|socket| socket.local_addr());
    if let Ok(address) = lan
        && !address.ip().is_loopback()
        && !address.ip().is_unspecified()
    {
        let refused = std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::new(address.ip(), harness.port),
            Duration::from_secs(2),
        );
        assert!(refused.is_err(), "reachable on {address}");
    }
}

#[test]
fn stopping_closes_the_port_at_once_and_it_can_be_reused() {
    let mut harness = Harness::start(&[("c1", "tok", BOTH)]);
    let port = harness.port;
    assert_eq!(
        rpc(&harness.url(), Some("tok"), "tools/list", json!({})).status,
        200
    );
    harness.stop_server();
    assert_eq!(
        harness.shared.status(),
        crate::mcp::state::McpStatus::Disabled
    );
    assert_eq!(
        rpc(
            &format!("http://127.0.0.1:{port}/mcp"),
            Some("tok"),
            "tools/list",
            json!({})
        )
        .status,
        0
    );
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
}

#[test]
fn a_port_conflict_is_reported_not_swallowed() {
    let blocker = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = blocker.local_addr().unwrap().port();
    let shared = McpShared::new();
    let (bridge, _inbox) = bridge::channel(shared.clone(), Arc::new(|| {}));
    let error = McpServer::start(shared.clone(), bridge, port)
        .err()
        .expect("must fail");
    assert!(error.contains("already in use"), "{error}");
    assert!(
        matches!(shared.status(), crate::mcp::state::McpStatus::Failed(message) if message.contains("already in use"))
    );
}
