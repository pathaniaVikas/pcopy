use std::{
    fmt::Display,
    io::Cursor,
    net::{IpAddr, Ipv4Addr},
};

use bytes::{Buf, BufMut, BytesMut};

use crate::relay::{
    peer::{PeerId, PeerInfo, PEER_ID_LENGTH_BYTES},
    relay::HealthStatus,
};

// Error for Frame operation
#[derive(Debug)]
pub enum Error {
    // Cannot read the wholr Frame. see [Frame] below
    Incomplete,
    Other(std::io::Error),
}

impl std::error::Error for Error {}

impl Into<std::io::Error> for Error {
    fn into(self) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::Other, self.to_string())
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Incomplete => write!(f, "{}", "Incomplete Frame Received")?,
            Error::Other(error) => write!(f, "{}", error.to_string())?,
        }
        Ok(())
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        match value.kind() {
            std::io::ErrorKind::InvalidData => Error::Incomplete,
            _ => Error::Other(value),
        }
    }
}

pub trait Operations {
    fn to_u8(&self) -> u8;
}

/// API operation
/// Relay Server accepts only requests starting with these operations.
/// Usually every request can be described as
/// |operation| body  ...
/// | 1 byte  | n|0 bytes ...
///
/// Multiple operations need to be piped together to create a successful connection between peers
#[derive(Debug, Clone, PartialEq)]
pub enum RelayOperations {
    // 01
    // Register peer with server, so that other peers can reach it.
    Register = 01,
    // 02
    // Probe the peer and get details to connect to it
    Probe = 02,
    // 03
    // Tell peer to initiate connection. See [RelayServer].
    DoConnect = 03,
    // 04
    // Ping Relay Server to get Round Trip Time
    Ping = 04,
    // 05
    // Ping Relay Server to get Round Trip Time
    ProbeAck = 05,
}

impl Operations for RelayOperations {
    fn to_u8(&self) -> u8 {
        match self {
            RelayOperations::Register => 01,
            RelayOperations::Probe => 02,
            RelayOperations::DoConnect => 03,
            RelayOperations::Ping => 04,
            RelayOperations::ProbeAck => 05,
        }
    }
}

impl TryFrom<u8> for RelayOperations {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            01 => Ok(RelayOperations::Register),
            02 => Ok(RelayOperations::Probe),
            03 => Ok(RelayOperations::DoConnect),
            04 => Ok(RelayOperations::Ping),
            05 => Ok(RelayOperations::ProbeAck),
            _ => Err(Error::Other(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Receiving Operation not supported",
            ))),
        }
    }
}

/// Operations used by Relay Server to send data to peers, eg. responses
#[derive(Debug, Clone, PartialEq)]
pub enum PeerOperations {
    // ========================== SOURCE PEER ==========================
    // Response status we send to source peer
    // 21
    // Register peer with server, so that other peers can reach it.
    RegisterResult = 21,
    // 22
    // Send Destination PeerInfo to source peer
    ProbeResult = 22,
    // 23
    // Send synchronization result to source peer
    DoConnectResult = 23,
    // 24
    // Send Ping reult
    PingResult = 24,
    // ========================== DESTINATION PEER ==========================
    // 25
    // Send commands to destination peer to fetch rtt or telling it to start hole punching request
    Commands = 25,
}

impl Operations for PeerOperations {
    fn to_u8(&self) -> u8 {
        match self {
            PeerOperations::RegisterResult => 21,
            PeerOperations::ProbeResult => 22,
            PeerOperations::DoConnectResult => 23,
            PeerOperations::PingResult => 24,
            PeerOperations::Commands => 25,
        }
    }
}

impl TryFrom<u8> for PeerOperations {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            21 => Ok(PeerOperations::RegisterResult),
            22 => Ok(PeerOperations::ProbeResult),
            23 => Ok(PeerOperations::DoConnectResult),
            24 => Ok(PeerOperations::PingResult),
            25 => Ok(PeerOperations::Commands),
            _ => Err(Error::Other(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Sending Operation not supported",
            ))),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum StatusCodes {
    SUCCESS = 200,
    FAILURE = 111,
}

impl StatusCodes {
    pub fn into_u8(self) -> u8 {
        match self {
            StatusCodes::SUCCESS => StatusCodes::SUCCESS as u8,
            StatusCodes::FAILURE => StatusCodes::FAILURE as u8,
        }
    }
}

impl TryFrom<u8> for StatusCodes {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            200 => Ok(StatusCodes::SUCCESS),
            111 => Ok(StatusCodes::FAILURE),
            _ => Err(Error::Other(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Status Code not supported",
            ))),
        }
    }
}

const SYNC_CODE: u8 = 100;
const DOCONNECT_CODE: u8 = 101;

#[derive(Clone, Eq, PartialEq)]
pub enum PeerCommands {
    SYNC,
    DOCONNECT(IpAddr),
}

impl PeerCommands {
    pub fn to_be_bytes(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PEER_ID_LENGTH_BYTES);
        match self {
            PeerCommands::SYNC => bytes.push(SYNC_CODE),
            PeerCommands::DOCONNECT(source_ip) => {
                bytes.push(DOCONNECT_CODE);
                bytes.extend_from_slice(source_ip.as_octets());
            }
        }
        return bytes;
    }
}

pub trait Frame<O, E>
where
    O: Operations,
    E: std::error::Error,
{
    // Reads operation from buf, and maps it to Operations allowed for that Frame
    fn read_operation(buf: &mut Cursor<&[u8]>) -> Result<O, E>;

    // Reads buf, and maps it to Frame of type F
    fn parse(buf: &mut Cursor<&[u8]>) -> Result<Self, E>
    where
        Self: Sized;

    // Writes operation and then Frame payload into a vector of bytes
    fn to_be_bytes(self) -> BytesMut;
}

// Frames sent from Peers to Relay Server
#[derive(Eq, PartialEq)]
pub enum RelayFrames {
    Register(PeerId),
    Probe(PeerId),
    DoConnect(PeerId),
    Ping,
    ProbeAck,
}

impl Frame<RelayOperations, Error> for RelayFrames {
    /// Advances cursor by OPERATION_LENGTH bytes
    fn read_operation(buf: &mut Cursor<&[u8]>) -> Result<RelayOperations, Error> {
        if !buf.has_remaining() {
            return Err(Error::Incomplete);
        }

        // Cursor is advanced by 1 byte here
        Ok(RelayOperations::try_from(buf.get_u8())?)
    }

    /// Advances cusror by Frame length
    fn parse(buf: &mut Cursor<&[u8]>) -> Result<RelayFrames, Error> {
        match Self::read_operation(buf)? {
            RelayOperations::Register => Ok(RelayFrames::Register(PeerId::try_from(buf)?)),
            RelayOperations::Probe => Ok(RelayFrames::Probe(PeerId::try_from(buf)?)),
            RelayOperations::DoConnect => Ok(RelayFrames::DoConnect(PeerId::try_from(buf)?)),
            RelayOperations::Ping => Ok(RelayFrames::Ping),
            RelayOperations::ProbeAck => Ok(RelayFrames::ProbeAck),
        }
    }

    fn to_be_bytes(self) -> BytesMut {
        let mut bytes = BytesMut::with_capacity(1024);

        match self {
            RelayFrames::Register(peer_id) => {
                bytes.put_u8(RelayOperations::Register.to_u8());
                bytes.extend_from_slice(&peer_id.as_bytes());
            }
            RelayFrames::Probe(peer_id) => {
                bytes.put_u8(RelayOperations::Probe.to_u8());
                bytes.extend_from_slice(&peer_id.as_bytes());
            }
            RelayFrames::DoConnect(peer_id) => {
                bytes.put_u8(RelayOperations::DoConnect.to_u8());
                bytes.extend_from_slice(&peer_id.as_bytes());
            }
            RelayFrames::Ping => {
                bytes.put_u8(RelayOperations::Ping.to_u8());
            }
            RelayFrames::ProbeAck => {
                bytes.put_u8(RelayOperations::ProbeAck.to_u8());
            }
        }
        bytes
    }
}

/// Frames sent to peers from Relay Server
#[derive(Clone, Eq, PartialEq)]
pub enum PeerFrames {
    RegisterResult(StatusCodes),
    ProbeResult(Option<PeerInfo>),
    DoConnectResult(StatusCodes),
    PingResult(HealthStatus),
    Command(PeerCommands),
}

impl Frame<PeerOperations, Error> for PeerFrames {
    fn read_operation(buf: &mut Cursor<&[u8]>) -> Result<PeerOperations, Error> {
        if !buf.has_remaining() {
            return Err(Error::Incomplete);
        }

        // Cursor is advanced by 1 byte here
        Ok(PeerOperations::try_from(buf.get_u8())?)
    }

    // TODO
    fn parse(buf: &mut Cursor<&[u8]>) -> Result<PeerFrames, Error> {
        match Self::read_operation(buf)? {
            PeerOperations::RegisterResult => {
                if !buf.has_remaining() {
                    return Err(Error::Incomplete);
                }
                Ok(PeerFrames::RegisterResult(StatusCodes::try_from(
                    buf.get_u8(),
                )?))
            }
            PeerOperations::ProbeResult => {
                if !buf.has_remaining() {
                    return Err(Error::Incomplete);
                }
                let status_code = StatusCodes::try_from(buf.get_u8())?;
                if status_code == StatusCodes::FAILURE {
                    return Ok(PeerFrames::ProbeResult(None));
                }
                // If status code is SUCCESS, read PeerInfo
                Ok(PeerFrames::ProbeResult(Some(PeerInfo::try_from(buf)?)))
            }
            PeerOperations::DoConnectResult => {
                if !buf.has_remaining() {
                    return Err(Error::Incomplete);
                }
                Ok(PeerFrames::DoConnectResult(StatusCodes::try_from(
                    buf.get_u8(),
                )?))
            }
            PeerOperations::PingResult => {
                if !buf.has_remaining() {
                    return Err(Error::Incomplete);
                }
                Ok(PeerFrames::PingResult(HealthStatus::try_from(
                    buf.get_u8(),
                )?))
            }
            PeerOperations::Commands => {
                if !buf.has_remaining() {
                    return Err(Error::Incomplete);
                }
                let command_code = buf.get_u8();
                match command_code {
                    SYNC_CODE => Ok(PeerFrames::Command(PeerCommands::SYNC)),
                    DOCONNECT_CODE => {
                        if buf.remaining() < 4 {
                            return Err(Error::Incomplete);
                        }
                        // Read 4 bytes for IPv4 address
                        // If you are using IPv6, you would read 16 bytes instead,
                        // TODO: Add support for IPv6
                        // Assuming the IP is in big-endian format
                        let ip_bytes = buf.get_u32(); // Assuming 4 bytes for IPv4
                        let ip = IpAddr::from(Ipv4Addr::new(
                            ((ip_bytes >> 24) & 0xFF) as u8,
                            ((ip_bytes >> 16) & 0xFF) as u8,
                            ((ip_bytes >> 8) & 0xFF) as u8,
                            (ip_bytes & 0xFF) as u8,
                        ));
                        Ok(PeerFrames::Command(PeerCommands::DOCONNECT(ip)))
                    }
                    _ => Err(Error::Other(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Unknown Peer Command",
                    ))),
                }
            }
        }
    }

    fn to_be_bytes(self) -> BytesMut {
        let mut bytes = BytesMut::with_capacity(1024);
        match self {
            PeerFrames::RegisterResult(status_codes) => {
                bytes.put_u8(PeerOperations::RegisterResult.to_u8());
                bytes.put_u8(status_codes.clone().into_u8());
            }
            PeerFrames::ProbeResult(peer_info) => {
                bytes.put_u8(PeerOperations::ProbeResult.to_u8());
                if peer_info.is_none() {
                    bytes.put_u8(StatusCodes::FAILURE.into_u8());
                    return bytes;
                }
                bytes.put_u8(StatusCodes::SUCCESS.into_u8());
                // PeerInfo is Some
                let peer_info = peer_info.unwrap();
                // PeerInfo is (SocketAddr, Option<PeerId>) where SocketAddr is (IpAddr, u16)
                // let peer_info = peer_info.unwrap();
                bytes.extend_from_slice(peer_info.to_be_bytes().as_ref());
            }
            PeerFrames::Command(peer_commands) => {
                bytes.put_u8(PeerOperations::Commands.to_u8());
                bytes.extend_from_slice(peer_commands.to_be_bytes().as_ref());
            }
            PeerFrames::DoConnectResult(status_codes) => {
                bytes.put_u8(PeerOperations::DoConnectResult.to_u8());
                bytes.put_u8(status_codes.clone().into_u8());
            }
            PeerFrames::PingResult(health_status) => {
                bytes.put_u8(PeerOperations::PingResult.to_u8());
                bytes.put_u8(health_status as u8);
            }
        }

        bytes
    }
}

// ...existing code...

#[cfg(test)]
mod Tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_read_operation_receiving_frame() {
        // Test for each ReceivingOperations variant
        let ops = [
            (01, RelayOperations::Register),
            (02, RelayOperations::Probe),
            (03, RelayOperations::DoConnect),
            (04, RelayOperations::Ping),
        ];

        for (byte, expected_op) in ops.iter() {
            let bytes = [*byte as u8];
            let mut buf = Cursor::new(&bytes[..]);
            let op = RelayFrames::read_operation(&mut buf).unwrap();
            assert_eq!(&op, expected_op);
        }

        // Test for incomplete buffer
        let empty: &[u8] = &[];
        let mut empty_buf = Cursor::new(empty);
        let err = RelayFrames::read_operation(&mut empty_buf).unwrap_err();
        matches!(err, Error::Incomplete);

        // Test for unsupported operation
        let invalid_bytes: &[u8] = &[0xFF];
        let mut invalid_buf = Cursor::new(invalid_bytes);
        let err = RelayFrames::read_operation(&mut invalid_buf).unwrap_err();
        matches!(err, Error::Other(_));
    }

    // ...existing code...

    #[test]
    fn test_read_operation_sending_frame() {
        // Test for each SendingOperations variant
        let ops = [
            (21, PeerOperations::RegisterResult),
            (22, PeerOperations::ProbeResult),
            (23, PeerOperations::DoConnectResult),
            (24, PeerOperations::PingResult),
            (25, PeerOperations::Commands),
        ];

        for (byte, expected_op) in ops.iter() {
            let bytes = [*byte as u8];
            let mut buf = Cursor::new(&bytes[..]);
            let op = PeerFrames::read_operation(&mut buf).unwrap();
            assert_eq!(&op, expected_op);
        }

        // Test for incomplete buffer
        let empty: &[u8] = &[];
        let mut empty_buf = Cursor::new(empty);
        let err = PeerFrames::read_operation(&mut empty_buf).unwrap_err();
        matches!(err, Error::Incomplete);

        // Test for unsupported operation
        let invalid_bytes: &[u8] = &[0xFF];
        let mut invalid_buf = Cursor::new(invalid_bytes);
        let err = PeerFrames::read_operation(&mut invalid_buf).unwrap_err();
        matches!(err, Error::Other(_));
    }

    #[test]
    fn test_to_be_bytes_sending_frame() {
        // Example: RegisterResult with StatusCodes::SUCCESS
        let operation = PeerOperations::RegisterResult;
        let frame = PeerFrames::RegisterResult(StatusCodes::SUCCESS);
        let bytes = frame.to_be_bytes();
        // Should start with operation byte, then frame bytes
        assert_eq!(bytes[0], operation as u8);
        assert_eq!(bytes[1], StatusCodes::SUCCESS.into_u8());

        // Example: PingResult with HealthStatus = 1 (assuming HealthStatus is u8)
        let operation = PeerOperations::PingResult;
        let frame = PeerFrames::PingResult(HealthStatus::HEALTHY);
        let bytes = frame.to_be_bytes();
        assert_eq!(bytes[0], operation as u8);
        assert_eq!(bytes[1], HealthStatus::HEALTHY as u8);

        // Example: Command with PeerCommands::SYNC
        let operation = PeerOperations::Commands;
        let frame = PeerFrames::Command(PeerCommands::SYNC);
        let bytes = frame.to_be_bytes();
        assert_eq!(bytes[0], operation as u8);
        assert_eq!(bytes[1], SYNC_CODE);

        // Example: DoConnectResult with StatusCodes::FAILURE
        let operation = PeerOperations::DoConnectResult;
        let frame = PeerFrames::DoConnectResult(StatusCodes::FAILURE);
        let bytes = frame.to_be_bytes();
        assert_eq!(bytes[0], operation as u8);
        assert_eq!(bytes[1], StatusCodes::FAILURE.into_u8());

        // Example: ProbeResult with StatusCodes::SUCCESS and None PeerInfo
        let operation = PeerOperations::ProbeResult;
        let frame = PeerFrames::ProbeResult(None);
        let bytes = frame.to_be_bytes();
        assert_eq!(bytes[0], operation as u8);
        assert_eq!(bytes[1], StatusCodes::FAILURE.into_u8());
        assert_eq!(bytes.len(), 2);

        // Example: ProbeResult with StatusCodes::SUCCESS and Some PeerInfo
        // You may need to construct a valid PeerInfo for your implementation
        // Here we assume PeerInfo::default() exists for demonstration
        let dummy_peer_info = PeerInfo::default(); // Replace with actual PeerInfo if needed
        let operation = PeerOperations::ProbeResult;
        let frame = PeerFrames::ProbeResult((Some(dummy_peer_info.clone())));
        let bytes = frame.to_be_bytes();
        assert_eq!(bytes[0], operation as u8);
        assert_eq!(bytes[1], StatusCodes::SUCCESS.into_u8());
        let expected_peer_info_bytes = dummy_peer_info.to_be_bytes();
        assert_eq!(&bytes[2..], &expected_peer_info_bytes[..]);
    }
}
