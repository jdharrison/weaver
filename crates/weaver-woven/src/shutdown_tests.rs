//! Verified remote composition on ephemeral loopback only, never a cloud target.
use super::*;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use woven_server::{
    ManagedServer, ManagedServerConfig, RemoteServerConfig, start_managed, start_remote,
};

// Public, throwaway test identity. Not a production secret or trust anchor.
const CERT: &str = include_str!("testdata/cert.pem.txt");
const KEY: &str = include_str!("testdata/key.pem.txt");
const TOKEN: &str = "weaver-test-only-static-credential-0123456789";
const MANAGED_TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ADMIN_TOKEN: &str = "weaver-managed-admin-test-credential";
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "weaver-shutdown-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let fixture = Self(path);
        for (name, value) in [("cert.pem", CERT), ("key.pem", KEY), ("token", TOKEN)] {
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
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn secure_remote_stop_notifies_peer_with_separate_runtimes() {
    let fixture = Fixture::new();
    let server_runtime = Runtime::new().unwrap();
    let server = server_runtime
        .block_on(start_remote(RemoteServerConfig {
            quic_bind_address: "127.0.0.1:0".parse().unwrap(),
            management_bind_address: "127.0.0.1:0".parse().unwrap(),
            certificate_file: fixture.0.join("cert.pem"),
            private_key_file: fixture.0.join("key.pem"),
            auth_token_file: fixture.0.join("token"),
        }))
        .unwrap();
    let config = WovenConfig {
        mode: ConnectivityMode::RemoteQuic,
        endpoint: Some(format!("quic://{}", server.quic_address)),
        ca_pem_file: Some(fixture.0.join("cert.pem")),
        token_file: Some(fixture.0.join("token")),
        run_deadline: Some(Instant::now() + Duration::from_secs(20)),
        ..WovenConfig::default()
    };
    let mut observer = WovenAdapter::new(config.clone()).unwrap();
    observer.start().unwrap();

    // Each adapter owns a distinct multi-thread runtime; the server survives stop.
    // Exercise normal and already-expired traffic deadlines, from sync and both
    // Tokio runtime flavors (including Drop's implicit stop).
    for expired in [false, true] {
        for context in 0..3 {
            let mut departing = WovenAdapter::new(config.clone()).unwrap();
            departing.start().unwrap();
            let entity = departing.entity_id().unwrap();
            if expired {
                departing.config.run_deadline = Some(Instant::now());
                assert!(departing.drain_envelopes().is_err());
            }
            let started = Instant::now();
            match context {
                0 => {
                    departing.stop();
                    assert_eq!(departing.status(), WovenStatus::Stopped);
                    assert!(departing.runtime.is_none());
                    assert!(departing.client.is_none());
                    assert!(departing.entity_id().is_none());
                    departing.stop(); // Idempotent.
                }
                1 => {
                    let caller = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    caller.block_on(async { departing.stop() });
                }
                _ => {
                    let caller = Runtime::new().unwrap();
                    caller.block_on(async { drop(departing) });
                }
            }
            assert!(started.elapsed() < CLOSE_TIMEOUT + Duration::from_millis(500));
            let observer_runtime = observer.runtime.as_ref().unwrap();
            let observer_client = observer.client.as_mut().unwrap();
            observer_runtime.block_on(async {
                let left = tokio::time::timeout(Duration::from_millis(500), async {
                    // Bound message count as well as elapsed time. Ignore unrelated controls.
                    for _ in 0..16 {
                        let envelope = observer_client.recv().await.unwrap();
                        if matches!(
                            envelope.message,
                            MessagePayload::Control(ControlPayload::EntityLeft(_))
                        ) && envelope.entity_id == Some(entity)
                        {
                            return true;
                        }
                    }
                    false
                })
                .await
                .expect("EntityLeft must arrive promptly, not at the QUIC idle timeout");
                assert!(left);
            });
            assert!(started.elapsed() < Duration::from_secs(3));
        }
    }
    observer.stop();
    drop(server);
}

#[test]
fn managed_adapter_admits_subscribes_publishes_and_releases_ccu() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("token"), MANAGED_TOKEN).unwrap();
    std::fs::write(fixture.0.join("admin"), ADMIN_TOKEN).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            fixture.0.join("admin"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    let server_runtime = Runtime::new().unwrap();
    let server = server_runtime
        .block_on(start_managed(ManagedServerConfig {
            quic_bind_address: "127.0.0.1:0".parse().unwrap(),
            management_bind_address: "127.0.0.1:0".parse().unwrap(),
            admin_bind_address: "127.0.0.1:0".parse().unwrap(),
            certificate_file: fixture.0.join("cert.pem"),
            private_key_file: fixture.0.join("key.pem"),
            admin_token_file: fixture.0.join("admin"),
            webtransport: None,
        }))
        .unwrap();
    server_runtime.block_on(provision(&server, 11, 17));

    let config = WovenConfig {
        mode: ConnectivityMode::ManagedQuic,
        endpoint: Some(format!("quic://{}", server.quic_address)),
        ca_pem_file: Some(fixture.0.join("cert.pem")),
        token_file: Some(fixture.0.join("token")),
        run_deadline: Some(Instant::now() + Duration::from_secs(20)),
        namespace_id: 11,
        session_id: 17,
        space_id: 1,
        space_epoch: 1,
        ..WovenConfig::default()
    };
    let mut adapter = WovenAdapter::new(config).unwrap();
    adapter.start().unwrap();
    assert_eq!(adapter.status(), WovenStatus::ManagedQuic);
    let entity = adapter.entity_id().unwrap();
    assert_eq!(server_runtime.block_on(active_ccu(&server, 11, 17)), 1);

    assert!(matches!(
        adapter.publish(
            2,
            None,
            &Payload {
                body: serde_json::json!({"managed": "wrong-policy"}),
                sequence: 1,
                revision: 1,
            },
            DeliveryClass::LatestValue,
            PersistenceClass::Stateful,
        ),
        Err(WovenAdapterError::UnsupportedChannelPolicy)
    ));
    adapter
        .publish(
            1,
            None,
            &Payload {
                body: serde_json::json!({"managed": true}),
                sequence: 1,
                revision: 1,
            },
            DeliveryClass::ReliableOrdered,
            PersistenceClass::Ephemeral,
        )
        .unwrap();
    let mut echoed = false;
    for _ in 0..50 {
        if adapter.drain_envelopes().unwrap().iter().any(|envelope| {
            envelope.channel == 1
                && envelope.entity == Some(entity)
                && envelope.sequence == 1
                && envelope.delivery == DeliveryClass::ReliableOrdered
                && envelope.persistence == PersistenceClass::Ephemeral
        }) {
            echoed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(echoed, "managed publish must be echoed through Woven");

    adapter.stop();
    let released = (0..20).any(|_| {
        if server_runtime.block_on(active_ccu(&server, 11, 17)) == 0 {
            true
        } else {
            std::thread::sleep(Duration::from_millis(50));
            false
        }
    });
    assert!(released, "managed disconnect must release allocated CCU");
}

async fn provision(server: &ManagedServer, namespace: u64, session: u64) {
    let body = json!({
        "revision": "1",
        "allocatedCCU": 1,
        "clientToken": MANAGED_TOKEN,
    })
    .to_string();
    let (status, _) = admin_request(
        server,
        "PUT",
        &format!("/v1/namespaces/{namespace}/sessions/{session}"),
        &body,
    )
    .await;
    assert_eq!(status, 201);
}

async fn active_ccu(server: &ManagedServer, namespace: u64, session: u64) -> u64 {
    let (status, body) = admin_request(
        server,
        "GET",
        &format!("/v1/namespaces/{namespace}/sessions/{session}"),
        "",
    )
    .await;
    assert_eq!(status, 200);
    body["admission"]["activeCCU"].as_u64().unwrap()
}

async fn admin_request(
    server: &ManagedServer,
    method: &str,
    path: &str,
    body: &str,
) -> (u16, Value) {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut stream = tokio::net::TcpStream::connect(server.admin_address)
            .await
            .unwrap();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\nAuthorization: Bearer {ADMIN_TOKEN}\r\nContent-Type: application/json\r\nWoven-Node-Incarnation: {}\r\n\r\n{body}",
            body.len(),
            server.node_incarnation
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.take(65_536).read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8(response).unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        let value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(body).unwrap()
        };
        (status, value)
    })
    .await
    .expect("bounded managed admin request")
}
