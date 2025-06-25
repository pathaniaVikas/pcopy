use std::{
    fmt::Display,
    io::{Cursor, Error, Read},
    net::IpAddr,
    sync::Arc,
    time::Duration,
};

use byteorder::ReadBytesExt;
use bytes::Buf;
use rand::{rand_core::le, RngCore};
use tokio::{io::BufWriter, net::TcpStream, sync::Mutex};
use tracing::info;

use crate::relay::{frame, relay};

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
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Default for PeerId {
    fn default() -> Self {
        let mut random_bytes: [u8; 64] = [0; 64];
        let mut rng = rand::rng();
        // Fill the array with random bytes
        rng.fill_bytes(&mut random_bytes);
        PeerId(random_bytes)
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
    type Error = frame::Error;

    fn try_from(value: &mut std::io::Cursor<&[u8]>) -> Result<Self, Self::Error> {
        if value.remaining() < PEER_ID_LENGTH_BYTES {
            return Err(frame::Error::Incomplete);
        }

        let start = value.position() as usize;
        let peer_id = PeerId::new(value.get_ref(), start);
        value.advance(peer_id.len());
        Ok(peer_id)
    }
}

const PEER_INFO_SERIALIZE_BYTES_LEN: usize = 33; // 1 byte for IP type + 32 bytes for IP and ping_rtt

/// Represents peer metadata
/// Ip: Peer public IP
/// ping_rtt: Round trip time it takes to reach to peer from Relay server
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub ip: IpAddr,
    pub ping_rtt: Duration,
}

impl Default for PeerInfo {
    fn default() -> Self {
        PeerInfo {
            peer_id: PeerId([0; PEER_ID_LENGTH_BYTES]),
            ip: IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            ping_rtt: Duration::from_millis(0),
        }
    }
}

impl PeerInfo {
    /// Convert struct to payload
    /// | ip_addr_type | ip Addr    | ping time |
    /// | 1 Byte       | 16 bytes   | 16 bytes  |
    ///
    /// Total - 33 bytes payload
    /// IPV4 address will be padded with 0s to make it 16 bytes
    /// IPV6 address will be 16 bytes
    /// ping_rtt will be converted to milliseconds and then to bytes
    pub fn to_be_bytes(&self) -> Vec<u8> {
        let mut bytes: Vec<u8> = Vec::with_capacity(PEER_INFO_SERIALIZE_BYTES_LEN);

        // Determine the IP type and append it
        match self.ip {
            IpAddr::V4(_) => bytes.push(0), // 0 for IPv4
            IpAddr::V6(_) => bytes.push(1), // 1 for IPv6
        }
        // Append the IP address bytes
        // IPv4 will be padded with 0s to make it 16 bytes
        // IPv6 will be 16 bytes
        let mut ip_bytes = match self.ip {
            IpAddr::V4(ip) => {
                let mut ip_bytes = vec![0; 16]; // Create a 16-byte vector
                ip_bytes[0..4].copy_from_slice(&ip.octets()); // Copy the 4 bytes of IPv4 address
                ip_bytes
            }
            IpAddr::V6(ip) => ip.octets().to_vec(),
        };
        bytes.append(&mut ip_bytes);
        bytes.append(&mut self.ping_rtt.as_millis().to_be_bytes().to_vec());

        bytes
    }
}

impl TryFrom<&mut Cursor<&[u8]>> for PeerInfo {
    type Error = frame::Error;

    fn try_from(value: &mut Cursor<&[u8]>) -> Result<Self, Self::Error> {
        if value.remaining() < PEER_INFO_SERIALIZE_BYTES_LEN {
            return Err(frame::Error::Incomplete);
        }

        // Read the first byte to determine IP type
        let ip_type = value.read_u8()?;
        let mut ip_bytes = [0; 16]; // 16 bytes for IP address
        value.read_exact(&mut ip_bytes)?;

        let ip = match ip_type {
            // 0 for IPv4
            0 => IpAddr::V4(std::net::Ipv4Addr::new(
                ip_bytes[0],
                ip_bytes[1],
                ip_bytes[2],
                ip_bytes[3],
            )),
            // 1 for IPv6
            1 => IpAddr::V6(std::net::Ipv6Addr::from(ip_bytes)),
            _ => {
                return Err(frame::Error::Other(Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid IP type",
                )))
            }
        };

        let ping_rtt_millis = u128::from_be_bytes(
            value.get_ref()[value.position() as usize..value.position() as usize + 16]
                .try_into()
                .unwrap(),
        );

        value.set_position(value.position() + 16); // Advance the cursor by 16 bytes for ping_rtt

        Ok(PeerInfo {
            peer_id: PeerId([0; PEER_ID_LENGTH_BYTES]), // PeerId is not included in this conversion
            ip,
            ping_rtt: Duration::from_millis(ping_rtt_millis as u64),
        })
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
