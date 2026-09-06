//! Broker health check via TCP connection

use std::net::TcpStream;
use std::time::Duration;

/// Check if the broker is healthy by attempting a TCP connection (async)
pub async fn health_check(addr: &str, timeout: Duration) -> bool {
    matches!(tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await, Ok(Ok(_)))
}

/// Check if the broker is healthy by attempting a TCP connection (synchronous)
pub fn health_check_sync(addr: &str, timeout: Duration) -> bool {
    TcpStream::connect_timeout(&addr.parse().unwrap_or_else(|_| "127.0.0.1:1883".parse().unwrap()), timeout)
        .is_ok()
}

/// Check whether a TCP address is free to bind (bind probe, not connect probe).
///
/// Briefly binds a listener and drops it. If any socket already listens on
/// the same address (including 0.0.0.0 wildcards), the bind fails. Used as a
/// pre-flight check so the harness can fail fast with a clear message instead
/// of waiting a full start timeout for a broker that can never bind.
pub fn port_free_sync(addr: &str) -> bool {
    std::net::TcpListener::bind(addr).is_ok()
}

/// Wait until the TCP address stops accepting connections, i.e. the previous
/// owner has released it (connection refused). Returns `false` on timeout.
pub fn wait_port_free_sync(addr: &str, timeout: Duration) -> bool {
    let peer = addr.parse().unwrap_or_else(|_| "127.0.0.1:1883".parse().unwrap());
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        // Connection refused -> nothing is listening on the port anymore.
        if TcpStream::connect_timeout(&peer, Duration::from_millis(200)).is_err() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}
