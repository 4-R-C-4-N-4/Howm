// P2P-CD TCP transport — Task 2.1
//
// Length-prefixed CBOR message framing over TCP inside the WireGuard tunnel.
// Wire format: 4-byte big-endian length header | CBOR payload
//
// The transport is bound to the WireGuard interface address on port 7654.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use p2pcd_types::ProtocolMessage;

/// Default P2P-CD protocol TCP port.
pub const DEFAULT_P2PCD_PORT: u16 = 7654;

/// Default read/write timeout.
const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(10);

/// A framed connection to one remote peer.
///
/// Messages are encoded as 4-byte big-endian length + CBOR payload.
pub struct P2pcdTransport {
    stream: TcpStream,
    io_timeout: Duration,
}

impl P2pcdTransport {
    /// Wrap an already-connected `TcpStream`.
    pub fn new(stream: TcpStream) -> Self {
        Self { stream, io_timeout: DEFAULT_IO_TIMEOUT }
    }

    /// Wrap with a custom I/O timeout.
    pub fn with_timeout(stream: TcpStream, io_timeout: Duration) -> Self {
        Self { stream, io_timeout }
    }

    /// Remote address of the peer.
    pub fn peer_addr(&self) -> Result<SocketAddr> {
        self.stream.peer_addr().context("get peer addr")
    }

    /// Send a `ProtocolMessage` — encodes to length-prefixed CBOR and flushes.
    pub async fn send(&mut self, msg: &ProtocolMessage) -> Result<()> {
        let encoded = msg.encode();
        let len = encoded.len() as u32;
        let header = len.to_be_bytes();

        timeout(self.io_timeout, async {
            self.stream.write_all(&header).await?;
            self.stream.write_all(&encoded).await?;
            self.stream.flush().await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("send timeout")?
        .context("send write")?;

        Ok(())
    }

    /// Receive a `ProtocolMessage` — reads the 4-byte length header then the CBOR payload.
    pub async fn recv(&mut self) -> Result<ProtocolMessage> {
        timeout(self.io_timeout, async {
            // Read the 4-byte length header
            let mut header = [0u8; 4];
            self.stream
                .read_exact(&mut header)
                .await
                .context("read length header")?;
            let len = u32::from_be_bytes(header) as usize;

            // Sanity cap: 1 MiB max message size
            if len > 1024 * 1024 {
                return Err(anyhow!("message too large: {} bytes", len));
            }

            // Read exactly `len` bytes of CBOR payload
            let mut buf = vec![0u8; len];
            self.stream
                .read_exact(&mut buf)
                .await
                .context("read CBOR payload")?;

            ProtocolMessage::decode(&mut buf.as_slice()).context("decode CBOR")
        })
        .await
        .context("recv timeout")?
    }

    /// Close the underlying TCP connection.
    pub async fn close(mut self) -> Result<()> {
        self.stream.shutdown().await.context("tcp shutdown")
    }
}

/// Accept loop: bind to `addr`, accept connections, and dispatch each to the
/// provided handler closure. Runs until the `shutdown` token is cancelled.
pub struct P2pcdListener {
    pub local_addr: SocketAddr,
    listener: TcpListener,
}

impl P2pcdListener {
    /// Bind to the given address.
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind(addr)
            .await
            .with_context(|| format!("bind P2PCD listener on {}", addr))?;
        let local_addr = listener.local_addr()?;
        tracing::info!("P2P-CD TCP listener on {}", local_addr);
        Ok(Self { local_addr, listener })
    }

    /// Accept one incoming connection, returning a `P2pcdTransport`.
    pub async fn accept(&self) -> Result<(P2pcdTransport, SocketAddr)> {
        let (stream, remote_addr) = self.listener.accept().await.context("tcp accept")?;
        Ok((P2pcdTransport::new(stream), remote_addr))
    }
}

/// Connect outbound to a remote peer's P2P-CD port.
pub async fn connect(addr: SocketAddr) -> Result<P2pcdTransport> {
    let stream = timeout(DEFAULT_IO_TIMEOUT, TcpStream::connect(addr))
        .await
        .context("connect timeout")?
        .with_context(|| format!("TCP connect to {}", addr))?;
    Ok(P2pcdTransport::new(stream))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use p2pcd_types::{CloseReason, DiscoveryManifest};

    /// Spawn a listener on loopback, connect to it, exchange one message, verify round-trip.
    #[tokio::test]
    async fn send_recv_round_trip() {
        let listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr;

        // Acceptor side
        let accept_task = tokio::spawn(async move {
            let (mut server, _) = listener.accept().await.unwrap();
            let msg = server.recv().await.unwrap();
            // Echo back
            server.send(&msg).await.unwrap();
            msg
        });

        // Connector side
        let mut client = connect(addr).await.unwrap();
        let manifest = DiscoveryManifest {
            peer_id: [1u8; 32],
            sequence_num: 42,
            display_name: "test-node".to_string(),
            capabilities: vec![],
        };
        let outgoing = ProtocolMessage::Offer(manifest.clone());
        client.send(&outgoing).await.unwrap();

        let echoed = client.recv().await.unwrap();

        // Verify the message round-tripped correctly
        match echoed {
            ProtocolMessage::Offer(m) => {
                assert_eq!(m.peer_id, manifest.peer_id);
                assert_eq!(m.sequence_num, manifest.sequence_num);
                assert_eq!(m.display_name, manifest.display_name);
            }
            other => panic!("unexpected message: {:?}", other),
        }

        // Also verify what the server received
        let server_received = accept_task.await.unwrap();
        match server_received {
            ProtocolMessage::Offer(m) => assert_eq!(m.sequence_num, 42),
            other => panic!("server got wrong message: {:?}", other),
        }
    }

    #[tokio::test]
    async fn send_recv_close_message() {
        let listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr;

        let accept_task = tokio::spawn(async move {
            let (mut server, _) = listener.accept().await.unwrap();
            server.recv().await.unwrap()
        });

        let mut client = connect(addr).await.unwrap();
        let close_msg = ProtocolMessage::Close(CloseReason::Normal);
        client.send(&close_msg).await.unwrap();

        let received = accept_task.await.unwrap();
        assert!(
            matches!(received, ProtocolMessage::Close(CloseReason::Normal)),
            "expected Close(Normal), got {:?}", received
        );
    }

    #[tokio::test]
    async fn send_recv_ping_pong() {
        let listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr;

        let accept_task = tokio::spawn(async move {
            let (mut server, _) = listener.accept().await.unwrap();
            let msg = server.recv().await.unwrap();
            // Respond with Pong
            if matches!(msg, ProtocolMessage::Ping(_)) {
                server.send(&ProtocolMessage::Pong(999)).await.unwrap();
            }
        });

        let mut client = connect(addr).await.unwrap();
        client.send(&ProtocolMessage::Ping(999)).await.unwrap();
        let pong = client.recv().await.unwrap();
        assert!(matches!(pong, ProtocolMessage::Pong(999)));

        accept_task.await.unwrap();
    }

    #[tokio::test]
    async fn recv_timeout_on_no_data() {
        let listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr;

        // Accept but never send anything
        let _accept_task = tokio::spawn(async move {
            let (_server, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
        });

        let stream = TcpStream::connect(addr).await.unwrap();
        let mut client = P2pcdTransport::with_timeout(stream, Duration::from_millis(50));
        let result = client.recv().await;
        assert!(result.is_err(), "expected timeout error");
    }
}
