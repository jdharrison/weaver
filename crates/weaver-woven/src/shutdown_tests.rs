//! Verified remote composition on ephemeral loopback only, never a cloud target.
use super::*;
use std::{path::PathBuf, time::Instant};
use woven_server::{RemoteServerConfig, start_remote};

// Public, throwaway test identity. Not a production secret or trust anchor.
const CERT: &str = include_str!("testdata/cert.pem.txt");
const KEY: &str = include_str!("testdata/key.pem.txt");
const TOKEN: &str = "weaver-test-only-static-credential-0123456789";

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("weaver-shutdown-{}", std::process::id()));
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
