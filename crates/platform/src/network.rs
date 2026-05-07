use std::net::{SocketAddr, TcpListener};

pub const LOOPBACK_HOST: &str = "127.0.0.1";

pub fn bind_loopback_dynamic_port() -> std::io::Result<(TcpListener, SocketAddr)> {
    let listener = TcpListener::bind((LOOPBACK_HOST, 0))?;
    let addr = listener.local_addr()?;
    Ok((listener, addr))
}
