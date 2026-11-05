//! Where the connector's TLS door takes its connections from. A forwarder (`hq edge`) passes each
//! client's TLS bytes to this machine over iroh; the bridge process that holds those streams
//! (`hq bridge serve` or `run`, a separate process on a phone) connects here and pipes them. This
//! is the one place the door takes connections, and it is on whenever `mcp_connector.tls` is, not
//! only while a connection window is armed: Claude's calls and token refreshes for an existing
//! grant must work at any time, and the armed window guards `/oauth/authorize` alone.
//!
//! The address is a unix socket in a directory only this user can enter. A TCP port, even on
//! loopback, would be a door anyone on the machine could take first: whoever binds it receives the
//! forwarded bytes, including the CA's validation of the certificate, and could order a
//! certificate for the device's name and end TLS in its place. A loopback `ip:port` is still
//! possible as an explicit opt-in for a machine with one user.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use hq_core::config::DoorTarget;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;

use super::ConnectorTls;
use crate::WsState;

/// Pause after an accept error (out of file descriptors, for one) so the loop does not spin.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);
/// How long a failed bind waits before the next try: another process may be about to let go of
/// the address, and a door that is down for good would leave the connector dark with no sign.
const REBIND_WAIT: Duration = Duration::from_secs(5);

/// Starts the door when `mcp_connector.tls` is on and valid: the certificate loop and the accept
/// loop. Called once, from `create_router`, with the router the main listener serves, so
/// `hq start` and `hq web` get the same door. A refusal is logged and the door stays off; nothing
/// else about the server changes.
pub(crate) fn spawn_door(state: &Arc<WsState>, router: Router) {
    let Some(cfg) = state.connector_cfg.clone() else {
        return;
    };
    let settings = match cfg.tls_settings(state.web_auth_token.as_deref()) {
        Ok(Some(settings)) => settings,
        Ok(None) => return,
        Err(e) => return refuse(&e),
    };
    let tls = match ConnectorTls::for_state(state, router, &cfg) {
        Ok(Some(tls)) => tls,
        Ok(None) => return,
        Err(e) => return refuse(&e),
    };
    let target = settings.door_listen.resolve(&state.vault_path);
    let host = settings.host;
    tokio::spawn(async move {
        tracing::info!(host = %host, door = %target, "connector tls door: starting");
        serve_door(tls, target).await;
    });
}

fn refuse(why: &str) {
    tracing::error!("connector tls door stays off: {why}");
    eprintln!("connector TLS door not enabled: {why}");
}

/// Binds the door and serves it, trying again if the bind fails, for as long as the task lives.
/// The certificate loop runs from the first moment, since the CA's validation arrives on the door.
pub(crate) async fn serve_door(tls: Arc<ConnectorTls>, target: DoorTarget) {
    let renewing = tls.clone();
    tokio::spawn(async move { renewing.run().await });
    loop {
        let ended = match &target {
            #[cfg(unix)]
            DoorTarget::Unix(path) => match bind_unix(path) {
                Ok(listener) => {
                    accept_unix(&tls, listener).await;
                    Ok(())
                }
                Err(e) => Err(e),
            },
            #[cfg(not(unix))]
            DoorTarget::Unix(_) => Err(std::io::Error::other(
                "a unix socket door needs a unix machine; set door_listen to a loopback ip:port",
            )),
            DoorTarget::Tcp(addr) => match TcpListener::bind(addr).await {
                Ok(listener) => {
                    accept_tcp(&tls, listener).await;
                    Ok(())
                }
                Err(e) => Err(e),
            },
        };
        if let Err(e) = ended {
            tracing::error!(door = %target, "connector tls door: could not listen: {e}");
            eprintln!(
                "connector TLS door could not listen on {}: {e}; trying again",
                target
            );
        }
        tokio::time::sleep(REBIND_WAIT).await;
    }
}

fn spawn_serve<S>(tls: &Arc<ConnectorTls>, stream: S)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let tls = tls.clone();
    tokio::spawn(async move {
        if let Err(e) = tls.serve(stream).await {
            tracing::debug!("connector tls door: a connection ended: {e}");
        }
    });
}

pub(crate) async fn accept_tcp(tls: &Arc<ConnectorTls>, listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((tcp, peer)) => {
                // The door is bound to loopback, so this cannot fail; a surprise is dropped.
                if peer.ip().is_loopback() {
                    spawn_serve(tls, tcp);
                }
            }
            Err(e) => {
                tracing::warn!("connector tls door: accept failed: {e}");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
            }
        }
    }
}

#[cfg(unix)]
pub(crate) async fn accept_unix(tls: &Arc<ConnectorTls>, listener: tokio::net::UnixListener) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => spawn_serve(tls, stream),
            Err(e) => {
                tracing::warn!("connector tls door: accept failed: {e}");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
            }
        }
    }
}

/// Binds the door's socket through the shared private-socket rules. Only the default directory is
/// created; a directory the owner named must already exist.
#[cfg(unix)]
pub(crate) fn bind_unix(path: &std::path::Path) -> std::io::Result<tokio::net::UnixListener> {
    let default_dir = path
        .parent()
        .and_then(|d| d.file_name())
        .is_some_and(|n| n == hq_core::config::DOOR_DIR);
    hq_core::fs_private::bind_private_socket(path, default_dir)
}

#[cfg(all(test, unix))]
mod tests {
    use super::bind_unix;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[tokio::test]
    async fn the_default_door_makes_its_directory_private_and_its_socket_private() {
        let vault = tempfile::tempdir().unwrap();
        let door = vault.path().join("_data").join(hq_core::config::DOOR_DIR);
        std::fs::create_dir_all(vault.path().join("_data")).unwrap();
        let socket = door.join(hq_core::config::DOOR_SOCKET_FILE);
        let _listener = bind_unix(&socket).unwrap();
        assert_eq!(mode(&door), 0o700);
        assert_eq!(mode(&socket), 0o600);
    }
}
