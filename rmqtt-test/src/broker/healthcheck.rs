//! Broker health check via TCP connection

use std::net::TcpStream;
use std::time::Duration;

/// Check if the broker is healthy by attempting a TCP connection (async)
pub async fn health_check(addr: &str, timeout: Duration) -> bool {
    matches!(tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await, Ok(Ok(_)))
}

/// Check if the broker is healthy by attempting a TCP connection (synchronous)
pub fn health_check_sync(addr: &str, timeout: Duration) -> bool {
    TcpStream::connect_timeout(&addr.parse().unwrap_or_else(|_| "127.0.0.1:1883".parse().unwrap()), timeout).is_ok()
}
