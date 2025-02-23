/* This file is part of DarkFi (https://dark.fi)
 *
 * Copyright (C) 2020-2023 Dyne.org foundation
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of the
 * License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use std::{io, time::Duration};

use async_rustls::{TlsAcceptor, TlsStream};
use async_std::net::{SocketAddr, TcpListener as AsyncStdTcpListener, TcpStream};
use async_trait::async_trait;
use socket2::{Domain, Socket, TcpKeepalive, Type};
use url::Url;

use super::{PtListener, PtStream};
use crate::Result;

/// TCP Dialer implementation
#[derive(Debug, Clone)]
pub struct TcpDialer {
    /// TTL to set for opened sockets, or `None` for default.
    ttl: Option<u32>,
}

impl TcpDialer {
    /// Instantiate a new [`TcpDialer`] with optional TTL.
    pub(crate) async fn new(ttl: Option<u32>) -> Result<Self> {
        Ok(Self { ttl })
    }

    /// Internal helper function to create a TCP socket.
    async fn create_socket(&self, socket_addr: SocketAddr) -> io::Result<Socket> {
        let domain = if socket_addr.is_ipv4() { Domain::IPV4 } else { Domain::IPV6 };
        let socket = Socket::new(domain, Type::STREAM, Some(socket2::Protocol::TCP))?;

        if socket_addr.is_ipv6() {
            socket.set_only_v6(true)?;
        }

        if let Some(ttl) = self.ttl {
            socket.set_ttl(ttl)?;
        }

        socket.set_nodelay(true)?;
        let keepalive = TcpKeepalive::new().with_time(Duration::from_secs(20));
        socket.set_tcp_keepalive(&keepalive)?;
        socket.set_reuse_port(true)?;

        Ok(socket)
    }

    /// Internal dial function
    pub(crate) async fn do_dial(
        &self,
        socket_addr: SocketAddr,
        timeout: Option<Duration>,
    ) -> Result<TcpStream> {
        let socket = self.create_socket(socket_addr).await?;

        let connection = if timeout.is_some() {
            socket.connect_timeout(&socket_addr.into(), timeout.unwrap())
        } else {
            socket.connect(&socket_addr.into())
        };

        match connection {
            Ok(()) => {}
            Err(err) if err.raw_os_error() == Some(libc::EINPROGRESS) => {}
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(err) => return Err(err.into()),
        }

        socket.set_nonblocking(true)?;
        let stream = TcpStream::from(std::net::TcpStream::from(socket));
        Ok(stream)
    }
}

/// TCP Listener implementation
#[derive(Debug, Clone)]
pub struct TcpListener {
    /// Size of the listen backlog for listen sockets
    backlog: i32,
}

impl TcpListener {
    /// Instantiate a new [`TcpListener`] with given backlog size.
    pub async fn new(backlog: i32) -> Result<Self> {
        Ok(Self { backlog })
    }

    /// Internal helper function to create a TCP socket.
    async fn create_socket(&self, socket_addr: SocketAddr) -> io::Result<Socket> {
        let domain = if socket_addr.is_ipv4() { Domain::IPV4 } else { Domain::IPV6 };
        let socket = Socket::new(domain, Type::STREAM, Some(socket2::Protocol::TCP))?;

        if socket_addr.is_ipv6() {
            socket.set_only_v6(true)?;
        }

        socket.set_nodelay(true)?;
        let keepalive = TcpKeepalive::new().with_time(Duration::from_secs(20));
        socket.set_tcp_keepalive(&keepalive)?;
        socket.set_reuse_port(true)?;

        Ok(socket)
    }

    /// Internal listen function
    pub(crate) async fn do_listen(&self, socket_addr: SocketAddr) -> Result<AsyncStdTcpListener> {
        let socket = self.create_socket(socket_addr).await?;
        socket.bind(&socket_addr.into())?;
        socket.listen(self.backlog)?;
        socket.set_nonblocking(true)?;
        Ok(AsyncStdTcpListener::from(std::net::TcpListener::from(socket)))
    }
}

#[async_trait]
impl PtListener for AsyncStdTcpListener {
    async fn next(&self) -> Result<(Box<dyn PtStream>, Url)> {
        let (stream, peer_addr) = match self.accept().await {
            Ok((s, a)) => (s, a),
            Err(e) => return Err(e.into()),
        };

        let url = Url::parse(&format!("tcp://{}", peer_addr))?;
        Ok((Box::new(stream), url))
    }
}

#[async_trait]
impl PtListener for (TlsAcceptor, AsyncStdTcpListener) {
    async fn next(&self) -> Result<(Box<dyn PtStream>, Url)> {
        let (stream, peer_addr) = match self.1.accept().await {
            Ok((s, a)) => (s, a),
            Err(e) => return Err(e.into()),
        };

        let stream = self.0.accept(stream).await;

        let url = Url::parse(&format!("tcp+tls://{}", peer_addr))?;

        if let Err(e) = stream {
            return Err(e.into())
        }

        Ok((Box::new(TlsStream::Server(stream.unwrap())), url))
    }
}
