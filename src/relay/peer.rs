use std::{fmt::Display, io::Error, net::IpAddr, sync::Arc, time::Duration};

use bytes::Buf;
use tokio::{io::BufWriter, net::TcpStream, sync::Mutex};

pub const PEER_ID_LENGTH_BYTES: usize = 64;

///
#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct PeerId([u8; PEER_ID_LENGTH_BYTES]);

impl PeerId {
    pub fn new(buf: &[u8], start_index: usize) -> Self {
        PeerId(
            buf[start_index..start_index + PEER_ID_LENGTH_BYTES]
                .iter()
                .cloned()
                .collect::<Vec<u8>>()
                .try_into()
                .unwrap(),
        )
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", String::from_utf8_lossy(&self.0))?;
        Ok(())
    }
}

/// Will advance the cursor position if value is read
impl TryFrom<&mut std::io::Cursor<&[u8]>> for PeerId {
    type Error = std::io::Error;

    fn try_from(value: &mut std::io::Cursor<&[u8]>) -> Result<Self, Self::Error> {
        let remaining_bytes_to_read = value.get_ref().len() - value.position() as usize;

        if remaining_bytes_to_read < PEER_ID_LENGTH_BYTES {
            return Err(Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Cannot read PeerId, found less bytes than required: {PEER_ID_LENGTH_BYTES}"
                ),
            ));
        }

        let start = value.position() as usize;
        let peer_id = PeerId::new(value.get_ref(), start);
        value.advance(peer_id.len());
        Ok(peer_id)
    }
}

/// Represents peer metadata
/// Ip: Peer public IP
/// ping_rtt: Round trip time it takes to reach to peer from Relay server
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub ip: IpAddr,
    pub ping_rtt: Duration,
}

impl PeerInfo {
    /// Convert struct to payload
    /// | ip Addr | ping time |
    /// | 4 bytes | 16 bytes  |
    ///
    /// Total - 20 bytes palyload
    pub fn to_be_bytes(&self) -> Vec<u8> {
        let mut bytes: Vec<u8> = Vec::with_capacity(20);

        let mut ip_bytes = match self.ip {
            IpAddr::V4(ip) => ip.octets().to_vec(),
            IpAddr::V6(ip) => ip.octets().to_vec(),
        };
        bytes.append(&mut ip_bytes);
        bytes.append(&mut self.ping_rtt.as_millis().to_be_bytes().to_vec());

        bytes
    }
}

/// Details about peer connection.
/// tcp_stream: TCP socket (may be closed after two peers are connected)
/// ip_addr: Public Ip Address of peer, where it can be reached.
#[derive(Clone)]
pub struct PeerConnectionMetadata {
    pub stream: Arc<Mutex<BufWriter<TcpStream>>>,
    pub ip_addr: IpAddr,
}
