// P2P-CD TCP transport — Task 2.1
//
// Length-prefixed CBOR message framing over TCP inside the WireGuard tunnel.
// Wire format: 4-byte big-endian length header | CBOR payload
//
// Note: ProtocolMessage::encode() already prepends the 4-byte length prefix,
// so we write the full encoded bytes directly and read using read_exact.

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
/// Messages are encoded as 4-byte big-endian length + CBOR payload
/// (via `ProtocolMessage::encode()`).
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
    /// `ProtocolMessage::encode()` already includes the 4-byte length prefix.
    pub async fn send(&mut self, msg: &ProtocolMessage) -> Result<()> {
        let encoded = msg.encode(); // 4-byte len prefix + CBOR payload

        timeout(self.io_timeout, async {
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

            // Reconstruct the length-prefixed buffer and decode via ProtocolMessage::decode
            let mut full = Vec::with_capacity(4 + len);
            full.extend_from_slice(&header);
            full.extend_from_slice(&buf);
            ProtocolMessage::decode(&mut full.as_slice()).context("decode CBOR")
        })
        .await
        .context("recv timeout")?
    }

    /// Close the underlying TCP connection.
    pub async fn close(mut self) -> Result<()> {
        self.stream.shutdown().await.context("tcp shutdown")
    }

    /// Split this transport into a (send_tx, recv_rx) channel pair suitable
    /// for passing to `HeartbeatManager::spawn`.
    ///
    /// The returned task forwards outbound `ProtocolMessage`s from `send_tx`
    /// to the TCP stream and inbound messages from the TCP stream to `recv_rx`.
    /// Both tasks run until the underlying TCP connection closes or the
    /// channels are dropped.
    ///
    /// After calling this method the original `P2pcdTransport` is consumed —
    /// all I/O goes through the channels.
    pub fn into_channels(
        self,
    ) -> (
        tokio::sync::mpsc::Sender<ProtocolMessage>,
        tokio::sync::mpsc::Receiver<ProtocolMessage>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::sync::mpsc;

        let (send_tx, mut send_rx) = mpsc::channel::<ProtocolMessage>(64);
        let (recv_tx, recv_rx) = mpsc::channel::<ProtocolMessage>(64);

        let io_timeout = self.io_timeout;
        let stream = self.stream;
        let (mut read_half, mut write_half) = tokio::io::split(stream);

        // Writer task: forward messages from send_rx → TCP
        tokio::spawn(async move {
            while let Some(msg) = send_rx.recv().await {
                let encoded = msg.encode();
                if let Err(e) = tokio::time::timeout(
                    io_timeout,
                    async {
                        write_half.write_all(&encoded).await?;
                        write_half.flush().await?;
                        Ok::<(), std::io::Error>(())
                    },
                )
                .await
                {
                    tracing::debug!("p2pcd channel write error: {:?}", e);
                    break;
                }
            }
        });

        // Reader task: forward messages from TCP → recv_tx
        tokio::spawn(async move {
            loop {
                // Read 4-byte length header
                let mut header = [0u8; 4];
                let read_result = tokio::time::timeout(
                    io_timeout,
                    read_half.read_exact(&mut header),
                )
                .await;
                match read_result {
                    Ok(Ok(_)) => {}
                    _ => break,
                }

                let len = u32::from_be_bytes(header) as usize;
                if len > 1024 * 1024 {
                    break;
                }

                let mut buf = vec![0u8; len];
                let read_result = tokio::time::timeout(
                    io_timeout,
                    read_half.read_exact(&mut buf),
                )
                .await;
                match read_result {
                    Ok(Ok(_)) => {}
                    _ => break,
                }

                let mut full = Vec::with_capacity(4 + len);
                full.extend_from_slice(&header);
                full.extend_from_slice(&buf);
                match ProtocolMessage::decode(&mut full.as_slice()) {
                    Ok(msg) => {
                        if recv_tx.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::debug!("p2pcd channel decode error: {:?}", e);
                        break;
                    }
                }
            }
        });

        (send_tx, recv_rx)
    }
}

/// Accept loop: bind to `addr`, accept connections, and dispatch each to the
/// provided handler closure.
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
    use p2pcd_types::{
        CloseReason, DiscoveryManifest, ProtocolMessage, PROTOCOL_VERSION,
    };
    use std::collections::BTreeMap;

    fn make_manifest(id: u8) -> DiscoveryManifest {
        DiscoveryManifest {
            protocol_version: PROTOCOL_VERSION,
            peer_id: [id; 32],
            sequence_num: id as u64,
            capabilities: vec![],
            personal_hash: vec![0u8; 32],
            hash_algorithm: "sha-256".to_string(),
        }
    }

    /// Spawn a listener on loopback, connect to it, exchange one message, verify round-trip.
    #[tokio::test]
    async fn send_recv_offer_round_trip() {
        let listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr;

        let accept_task = tokio::spawn(async move {
            let (mut server, _) = listener.accept().await.unwrap();
            let msg = server.recv().await.unwrap();
            server.send(&msg).await.unwrap(); // echo back
            msg
        });

        let mut client = connect(addr).await.unwrap();
        let manifest = make_manifest(1);
        let outgoing = ProtocolMessage::Offer { manifest: manifest.clone() };
        client.send(&outgoing).await.unwrap();

        let echoed = client.recv().await.unwrap();
        match &echoed {
            ProtocolMessage::Offer { manifest: m } => {
                assert_eq!(m.peer_id, manifest.peer_id);
                assert_eq!(m.sequence_num, manifest.sequence_num);
            }
            other => panic!("unexpected message: {:?}", other),
        }

        let server_received = accept_task.await.unwrap();
        match server_received {
            ProtocolMessage::Offer { manifest: m } => assert_eq!(m.sequence_num, 1),
            other => panic!("server got wrong message: {:?}", other),
        }
    }

    #[tokio::test]
    async fn send_recv_confirm() {
        let listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr;

        let accept_task = tokio::spawn(async move {
            let (mut server, _) = listener.accept().await.unwrap();
            server.recv().await.unwrap()
        });

        let mut client = connect(addr).await.unwrap();
        let msg = ProtocolMessage::Confirm {
            personal_hash: vec![1u8; 32],
            active_set: vec!["p2pcd.social.post.1".to_string()],
            accepted_params: None,
        };
        client.send(&msg).await.unwrap();

        let received = accept_task.await.unwrap();
        match received {
            ProtocolMessage::Confirm { active_set, .. } => {
                assert_eq!(active_set, vec!["p2pcd.social.post.1"]);
            }
            other => panic!("expected Confirm, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn send_recv_close() {
        let listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr;

        let accept_task = tokio::spawn(async move {
            let (mut server, _) = listener.accept().await.unwrap();
            server.recv().await.unwrap()
        });

        let mut client = connect(addr).await.unwrap();
        let msg = ProtocolMessage::Close {
            personal_hash: vec![0u8; 32],
            reason: CloseReason::Normal,
        };
        client.send(&msg).await.unwrap();

        let received = accept_task.await.unwrap();
        assert!(
            matches!(received, ProtocolMessage::Close { reason: CloseReason::Normal, .. }),
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
            if let ProtocolMessage::Ping { timestamp } = msg {
                server.send(&ProtocolMessage::Pong { timestamp }).await.unwrap();
            }
        });

        let mut client = connect(addr).await.unwrap();
        client.send(&ProtocolMessage::Ping { timestamp: 12345 }).await.unwrap();
        let pong = client.recv().await.unwrap();
        assert!(matches!(pong, ProtocolMessage::Pong { timestamp: 12345 }));

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

    #[tokio::test]
    async fn confirm_with_accepted_params() {
        use p2pcd_types::ScopeParams;

        let listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr;

        let accept_task = tokio::spawn(async move {
            let (mut server, _) = listener.accept().await.unwrap();
            server.recv().await.unwrap()
        });

        let mut params = BTreeMap::new();
        params.insert(
            "p2pcd.social.post.1".to_string(),
            ScopeParams { rate_limit: 10, ttl: 3600 },
        );

        let mut client = connect(addr).await.unwrap();
        let msg = ProtocolMessage::Confirm {
            personal_hash: vec![2u8; 32],
            active_set: vec!["p2pcd.social.post.1".to_string()],
            accepted_params: Some(params),
        };
        client.send(&msg).await.unwrap();

        let received = accept_task.await.unwrap();
        match received {
            ProtocolMessage::Confirm { accepted_params: Some(p), .. } => {
                let scope = p.get("p2pcd.social.post.1").unwrap();
                assert_eq!(scope.rate_limit, 10);
                assert_eq!(scope.ttl, 3600);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }
}
