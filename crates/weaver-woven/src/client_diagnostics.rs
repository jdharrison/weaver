//! Log-safe client failures: never forward peer close text or credentials.

use crate::WovenAdapterError;
use std::fmt::Write as _;
use woven_client::{ClientError, QuicDiagnostics};

pub(super) fn client_error(error: ClientError) -> WovenAdapterError {
    WovenAdapterError::ClientFailed(client_message(error))
}

pub(super) fn connected_client_error(
    error: ClientError,
    operation: &'static str,
    diagnostics: Option<QuicDiagnostics>,
) -> WovenAdapterError {
    let is_transport = matches!(&error, ClientError::Transport(_));
    let mut message = format!("{operation}: {}", client_message(error));
    if is_transport && let Some(diagnostics) = diagnostics {
        let close = diagnostics.close_reason.map_or_else(
            || "no close reason observed (not a liveness guarantee)".to_owned(),
            |reason| reason.to_string(),
        );
        let budget = diagnostics.max_datagram_bytes.map_or_else(
            || "unsupported/disabled".to_owned(),
            |bytes| format!("{bytes} bytes"),
        );
        let _ = write!(message, "; QUIC {close}; datagram frame budget {budget}");
    }
    WovenAdapterError::ClientFailed(message)
}

fn client_message(error: ClientError) -> String {
    match error {
        ClientError::Transport(message) => transport_message(&message),
        ClientError::HandshakeFailed(_) => "handshake failed (details redacted)".to_owned(),
        ClientError::UnsupportedScheme(_) => "unsupported URL scheme".to_owned(),
        other => other.to_string(),
    }
}

fn transport_message(message: &str) -> String {
    // These are SDK-local errors. Parse numbers and reconstruct rather than echoing
    // arbitrary text; real QUIC closure classification comes from typed state above.
    if let Some(lengths) = message.strip_prefix("datagram frame length ")
        && let Some((actual, maximum)) = lengths.split_once(" exceeds current transport limit ")
        && let (Ok(actual), Ok(maximum)) = (actual.parse::<usize>(), maximum.parse::<usize>())
    {
        return format!(
            "datagram frame too large ({actual} bytes; transport limit {maximum} bytes)"
        );
    }
    match message {
        "datagrams are unsupported by the peer or disabled locally" => {
            "datagrams unsupported by peer or disabled locally".to_owned()
        }
        "control stream ended before a complete frame arrived" => {
            "control stream ended before a complete frame arrived".to_owned()
        }
        _ => "transport/TLS failure (details redacted)".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use woven_client::QuicCloseReason;

    #[test]
    fn only_numeric_datagram_limits_survive_redaction() {
        assert_eq!(
            transport_message("datagram frame length 1168 exceeds current transport limit 1100"),
            "datagram frame too large (1168 bytes; transport limit 1100 bytes)"
        );
        for message in [
            "token=SECRET\nforged log",
            "datagram frame length SECRET exceeds current transport limit 1100",
            "datagram frame length 1168 exceeds current transport limit 1100 SECRET",
            "datagram frame length 1168 exceeds current transport limit 1100\nSECRET",
            "datagram frame length -1 exceeds current transport limit 1100",
        ] {
            assert_eq!(
                transport_message(message),
                "transport/TLS failure (details redacted)"
            );
        }
    }

    #[test]
    fn typed_close_state_and_operation_are_visible_without_peer_text() {
        for reason in [
            QuicCloseReason::TimedOut,
            QuicCloseReason::ApplicationClosed { code: 7 },
            QuicCloseReason::ConnectionClosed { code: 10 },
            QuicCloseReason::Reset,
        ] {
            let error = connected_client_error(
                ClientError::Transport("token=SECRET\nforged log".into()),
                "receive reliable control",
                Some(QuicDiagnostics {
                    close_reason: Some(reason),
                    max_datagram_bytes: Some(1200),
                }),
            )
            .to_string();
            assert!(error.contains("receive reliable control"));
            assert!(error.contains(&reason.to_string()));
            assert!(error.contains("datagram frame budget 1200 bytes"));
            assert!(!error.contains("SECRET") && !error.contains('\n'));
        }
    }

    #[test]
    fn missing_close_or_datagram_support_never_claims_liveness() {
        let error = connected_client_error(
            ClientError::Transport(
                "datagrams are unsupported by the peer or disabled locally".into(),
            ),
            "publish positioned datagram",
            Some(QuicDiagnostics {
                close_reason: None,
                max_datagram_bytes: None,
            }),
        )
        .to_string();
        assert!(error.contains("datagrams unsupported"));
        assert!(error.contains("not a liveness guarantee"));
        assert!(error.contains("datagram frame budget unsupported/disabled"));
    }

    #[test]
    fn preconnection_failures_stay_redacted() {
        for error in [
            ClientError::Transport("SECRET".into()),
            ClientError::HandshakeFailed("SECRET".into()),
            ClientError::UnsupportedScheme("SECRET".into()),
        ] {
            assert!(!client_error(error).to_string().contains("SECRET"));
        }
    }
}
