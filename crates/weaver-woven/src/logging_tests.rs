//! Session logs over verified QUIC and the authenticated node feed on ephemeral loopback only.
use super::*;
use crate::{ConnectivityMode, WovenStatus};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::atomic::AtomicU64,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use woven_server::{ManagedServer, ManagedServerConfig, start_managed};

// Existing public throwaway TLS identity, never a production trust anchor or secret.
const CERT: &str = include_str!("testdata/cert.pem.txt");
const KEY: &str = include_str!("testdata/key.pem.txt");
const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ADMIN_TOKEN: &str = "weaver-logging-test-only-admin-token";
const MESSAGE: &str = "First-Person Lab: connected successfully; delayed client logging test (3 seconds after connection).";
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "weaver-logging-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let fixture = Self(path);
        for (name, value) in [
            ("cert.pem", CERT),
            ("key.pem", KEY),
            ("token", TOKEN),
            ("admin", ADMIN_TOKEN),
        ] {
            let path = fixture.0.join(name);
            std::fs::write(&path, value).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
        fixture
    }

    fn start(&self, runtime: &tokio::runtime::Runtime) -> ManagedServer {
        let server = runtime
            .block_on(start_managed(ManagedServerConfig {
                quic_bind_address: "127.0.0.1:0".parse().unwrap(),
                management_bind_address: "127.0.0.1:0".parse().unwrap(),
                admin_bind_address: "127.0.0.1:0".parse().unwrap(),
                certificate_file: self.0.join("cert.pem"),
                private_key_file: self.0.join("key.pem"),
                admin_token_file: self.0.join("admin"),
                webtransport: None,
            }))
            .unwrap();
        let body = json!({"revision": "1", "allocatedCCU": 1, "clientToken": TOKEN}).to_string();
        assert_eq!(
            runtime
                .block_on(admin_request(
                    &server,
                    "PUT",
                    "/v1/namespaces/1/sessions/1",
                    &body
                ))
                .0,
            201
        );
        server
    }

    fn config(&self, server: &ManagedServer) -> WovenConfig {
        WovenConfig {
            mode: ConnectivityMode::ManagedQuic,
            endpoint: Some(format!("quic://{}", server.quic_address)),
            ca_pem_file: Some(self.0.join("cert.pem")),
            token_file: Some(self.0.join("token")),
            namespace_id: 1,
            session_id: 1,
            space_id: 1,
            space_epoch: 1,
            run_deadline: Some(Instant::now() + Duration::from_secs(20)),
            ..WovenConfig::default()
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn admin_request(
    server: &ManagedServer,
    method: &str,
    path: &str,
    body: &str,
) -> (u16, Value) {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut stream = tokio::net::TcpStream::connect(server.admin_address).await.unwrap();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\nAuthorization: Bearer {ADMIN_TOKEN}\r\nContent-Type: application/json\r\nWoven-Node-Incarnation: {}\r\n\r\n{body}",
            body.len(), server.node_incarnation
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.take(65_536).read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8(response).unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, serde_json::from_str(body).unwrap())
    }).await.expect("bounded local admin request")
}

#[test]
fn worker_info_log_reaches_authenticated_node_feed_once_and_preserves_chat() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fixture.start(&runtime);
    let mut driver = WovenRealtimeDriver::connect(fixture.config(&server)).unwrap();
    // Bypass the driver check to exercise an SDK-local logging error on the worker.
    driver
        .command_tx
        .try_send(WorkerCommand::LogInfo {
            message: String::new(),
        })
        .unwrap();
    driver.log_info(MESSAGE).unwrap();
    let mut events = Vec::new();
    driver.poll(
        &[RealtimeCommand::PublishReliable {
            sequence: 1,
            payload: "chat after logging".to_owned(),
        }],
        &mut events,
    );
    super::tests::poll_until(&mut driver, &mut events, |events| {
        events.iter().any(|event| matches!(event, RealtimeEvent::Payload { sequence: 1, payload, .. } if payload == "chat after logging"))
    });
    let (status, feed) = runtime.block_on(admin_request(
        &server,
        "GET",
        "/v1/logs?after=0&limit=32",
        "",
    ));
    assert_eq!(status, 200);
    let entries = feed["entries"].as_array().unwrap();
    let logs: Vec<_> = entries
        .iter()
        .filter(|entry| entry["event"] == "client.log")
        .collect();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0]["message"], MESSAGE);
    assert_eq!(logs[0]["level"], "info");
    assert_eq!(logs[0]["source"], "client");
    assert_eq!(logs[0]["namespaceId"], "1");
    assert_eq!(logs[0]["sessionId"], "1");
    let connected = entries
        .iter()
        .find(|entry| entry["event"] == "client.connected")
        .unwrap();
    assert_eq!(logs[0]["connectionId"], connected["connectionId"]);
    assert_eq!(feed["nodeIncarnation"], server.node_incarnation);
}

#[test]
fn adapter_sdk_validation_and_server_log_rejections_preserve_connection() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fixture.start(&runtime);
    let mut adapter = WovenAdapter::new(fixture.config(&server)).unwrap();
    assert!(matches!(
        adapter.log_info(MESSAGE),
        Err(WovenAdapterError::NotRunning)
    ));
    adapter.start().unwrap();
    for message in [String::new(), "x".repeat(1_025)] {
        assert!(adapter.log_info(&message).is_err());
        assert_eq!(adapter.status(), WovenStatus::ManagedQuic);
    }
    // Eleven bounded local sends deliberately exceed the ten/second logging cap.
    // Only ten may be captured; draining the rejection must still allow the chat echo.
    for _ in 0..11 {
        adapter.log_info("local rate-limit diagnostic").unwrap();
    }
    adapter
        .publish_envelope(
            None,
            PayloadEnvelope {
                body_json: "chat after rejected log".to_owned(),
                sequence: 1,
                revision: 0,
                channel: 1,
                entity: None,
                delivery: DeliveryClass::ReliableOrdered,
                persistence: PersistenceClass::Ephemeral,
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut received = false;
    while Instant::now() < deadline && !received {
        received = adapter.drain_envelopes().unwrap().iter().any(|envelope| {
            envelope.sequence == 1 && envelope.body_json == "chat after rejected log"
        });
    }
    assert!(
        received,
        "log-specific ProtocolError must not terminate reliable simulation traffic"
    );
    assert_eq!(adapter.status(), WovenStatus::ManagedQuic);
    let (status, feed) = runtime.block_on(admin_request(
        &server,
        "GET",
        "/v1/logs?after=0&limit=32",
        "",
    ));
    assert_eq!(status, 200);
    assert_eq!(
        feed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["event"] == "client.log")
            .count(),
        10
    );
    adapter.stop();
    assert!(matches!(
        adapter.log_info(MESSAGE),
        Err(WovenAdapterError::NotRunning)
    ));
}
