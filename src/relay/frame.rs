use std::{fmt::Display, io::Cursor, net::IpAddr};

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
/// Server accepts only requests starting with these operations.
/// Usually every request can be described as
/// |operation| body  ...
/// | 1 byte  | n|0 bytes ...
///
/// These operations need to be run in series to make connection succesfully between peers
#[derive(Debug, Clone, PartialEq)]
pub enum ReceivingOperations {
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

impl Operations for ReceivingOperations {
    fn to_u8(&self) -> u8 {
        match self {
            ReceivingOperations::Register => 01,
            ReceivingOperations::Probe => 02,
            ReceivingOperations::DoConnect => 03,
            ReceivingOperations::Ping => 04,
            ReceivingOperations::ProbeAck => 05,
        }
    }
}

impl TryFrom<u8> for ReceivingOperations {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            01 => Ok(ReceivingOperations::Register),
            02 => Ok(ReceivingOperations::Probe),
            03 => Ok(ReceivingOperations::DoConnect),
            04 => Ok(ReceivingOperations::Ping),
            05 => Ok(ReceivingOperations::ProbeAck),
            _ => Err(Error::Other(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Receiving Operation not supported",
            ))),
        }
    }
}

/// Operations used by Relay Server to send data to peers, eg. responses
#[derive(Debug, Clone, PartialEq)]
pub enum SendingOperations {
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

impl Operations for SendingOperations {
    fn to_u8(&self) -> u8 {
        match self {
            SendingOperations::RegisterResult => 21,
            SendingOperations::ProbeResult => 22,
            SendingOperations::DoConnectResult => 23,
            SendingOperations::PingResult => 24,
            SendingOperations::Commands => 25,
        }
    }
}

impl TryFrom<u8> for SendingOperations {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            21 => Ok(SendingOperations::RegisterResult),
            22 => Ok(SendingOperations::ProbeResult),
            23 => Ok(SendingOperations::DoConnectResult),
            24 => Ok(SendingOperations::PingResult),
            25 => Ok(SendingOperations::Commands),
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
    FAILURE = 199,
}

impl StatusCodes {
    pub fn into_u8(self) -> u8 {
        match self {
            StatusCodes::SUCCESS => StatusCodes::SUCCESS as u8,
            StatusCodes::FAILURE => StatusCodes::FAILURE as u8,
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

pub trait Frame<O, F, E>
where
    O: Operations,
    F: Frame<O, F, E>,
    E: std::error::Error,
{
    // Reads operation from buf, and maps it to Operations allowed for that Frame
    fn read_operation(buf: &mut Cursor<&[u8]>) -> Result<O, E>;

    // Reads buf, and maps it to Frame of type F
    fn parse(buf: &mut Cursor<&[u8]>) -> Result<F, E>;

    // Writes operation and then Frame payload into a vector of bytes
    fn frame_to_be_bytes(self) -> BytesMut;
}

// Payload for frame received by Relay Server usually requests from peers except ProbeAck which is Response from destination peer
#[derive(Eq, PartialEq)]
pub enum RecevingFrames {
    Register(PeerId),
    Probe(PeerId),
    DoConnect(PeerId),
    Ping,
    ProbeAck,
}

impl Frame<ReceivingOperations, RecevingFrames, Error> for RecevingFrames {
    /// Advances cursor by OPERATION_LENGTH bytes
    fn read_operation(buf: &mut Cursor<&[u8]>) -> Result<ReceivingOperations, Error> {
        if !buf.has_remaining() {
            return Err(Error::Incomplete);
        }

        // Cursor is advanced by 1 byte here
        Ok(ReceivingOperations::try_from(buf.get_u8())?)
    }

    /// Advances cusror by Frame length
    fn parse(buf: &mut Cursor<&[u8]>) -> Result<RecevingFrames, Error> {
        match Self::read_operation(buf)? {
            ReceivingOperations::Register => Ok(RecevingFrames::Register(PeerId::try_from(buf)?)),
            ReceivingOperations::Probe => Ok(RecevingFrames::Probe(PeerId::try_from(buf)?)),
            ReceivingOperations::DoConnect => Ok(RecevingFrames::DoConnect(PeerId::try_from(buf)?)),
            ReceivingOperations::Ping => Ok(RecevingFrames::Ping),
            ReceivingOperations::ProbeAck => Ok(RecevingFrames::ProbeAck),
        }
    }

    fn frame_to_be_bytes(self) -> BytesMut {
        let mut bytes = BytesMut::with_capacity(1024);

        match self {
            RecevingFrames::Register(peer_id) => {
                bytes.put_u8(ReceivingOperations::Register.to_u8());
                bytes.extend_from_slice(&peer_id.as_bytes());
            }
            RecevingFrames::Probe(peer_id) => {
                bytes.put_u8(ReceivingOperations::Probe.to_u8());
                bytes.extend_from_slice(&peer_id.as_bytes());
            }
            RecevingFrames::DoConnect(peer_id) => {
                bytes.put_u8(ReceivingOperations::DoConnect.to_u8());
                bytes.extend_from_slice(&peer_id.as_bytes());
            }
            RecevingFrames::Ping => {
                bytes.put_u8(ReceivingOperations::Ping.to_u8());
            }
            RecevingFrames::ProbeAck => {
                bytes.put_u8(ReceivingOperations::ProbeAck.to_u8());
            }
        }
        bytes
    }
}

// Frames sent to peers, eg. Response frames and Commands to destination peers
#[derive(Clone, Eq, PartialEq)]
pub enum SendingFrames {
    RegisterResult(StatusCodes),
    ProbeResult(Option<PeerInfo>),
    DoConnectResult(StatusCodes),
    PingResult(HealthStatus),
    Command(PeerCommands),
}

impl Frame<SendingOperations, SendingFrames, Error> for SendingFrames {
    fn read_operation(buf: &mut Cursor<&[u8]>) -> Result<SendingOperations, Error> {
        if !buf.has_remaining() {
            return Err(Error::Incomplete);
        }

        // Cursor is advanced by 1 byte here
        Ok(SendingOperations::try_from(buf.get_u8())?)
    }

    // TODO
    fn parse(buf: &mut Cursor<&[u8]>) -> Result<SendingFrames, Error> {
        match Self::read_operation(buf)? {
            SendingOperations::RegisterResult => todo!(),
            SendingOperations::ProbeResult => todo!(),
            SendingOperations::DoConnectResult => todo!(),
            SendingOperations::PingResult => todo!(),
            SendingOperations::Commands => todo!(),
        }
    }

    fn frame_to_be_bytes(self) -> BytesMut {
        let mut bytes = BytesMut::with_capacity(1024);
        match self {
            SendingFrames::RegisterResult(status_codes) => {
                bytes.put_u8(SendingOperations::RegisterResult.to_u8());
                bytes.put_u8(status_codes.clone().into_u8());
            }
            SendingFrames::ProbeResult(peer_info) => {
                bytes.put_u8(SendingOperations::ProbeResult.to_u8());
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
            SendingFrames::Command(peer_commands) => {
                bytes.put_u8(SendingOperations::Commands.to_u8());
                bytes.extend_from_slice(peer_commands.to_be_bytes().as_ref());
            }
            SendingFrames::DoConnectResult(status_codes) => {
                bytes.put_u8(SendingOperations::DoConnectResult.to_u8());
                bytes.put_u8(status_codes.clone().into_u8());
            }
            SendingFrames::PingResult(health_status) => {
                bytes.put_u8(SendingOperations::PingResult.to_u8());
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
            (01, ReceivingOperations::Register),
            (02, ReceivingOperations::Probe),
            (03, ReceivingOperations::DoConnect),
            (04, ReceivingOperations::Ping),
        ];

        for (byte, expected_op) in ops.iter() {
            let bytes = [*byte as u8];
            let mut buf = Cursor::new(&bytes[..]);
            let op = RecevingFrames::read_operation(&mut buf).unwrap();
            assert_eq!(&op, expected_op);
        }

        // Test for incomplete buffer
        let empty: &[u8] = &[];
        let mut empty_buf = Cursor::new(empty);
        let err = RecevingFrames::read_operation(&mut empty_buf).unwrap_err();
        matches!(err, Error::Incomplete);

        // Test for unsupported operation
        let invalid_bytes: &[u8] = &[0xFF];
        let mut invalid_buf = Cursor::new(invalid_bytes);
        let err = RecevingFrames::read_operation(&mut invalid_buf).unwrap_err();
        matches!(err, Error::Other(_));
    }

    // ...existing code...

    #[test]
    fn test_read_operation_sending_frame() {
        // Test for each SendingOperations variant
        let ops = [
            (21, SendingOperations::RegisterResult),
            (22, SendingOperations::ProbeResult),
            (23, SendingOperations::DoConnectResult),
            (24, SendingOperations::PingResult),
            (25, SendingOperations::Commands),
        ];

        for (byte, expected_op) in ops.iter() {
            let bytes = [*byte as u8];
            let mut buf = Cursor::new(&bytes[..]);
            let op = SendingFrames::read_operation(&mut buf).unwrap();
            assert_eq!(&op, expected_op);
        }

        // Test for incomplete buffer
        let empty: &[u8] = &[];
        let mut empty_buf = Cursor::new(empty);
        let err = SendingFrames::read_operation(&mut empty_buf).unwrap_err();
        matches!(err, Error::Incomplete);

        // Test for unsupported operation
        let invalid_bytes: &[u8] = &[0xFF];
        let mut invalid_buf = Cursor::new(invalid_bytes);
        let err = SendingFrames::read_operation(&mut invalid_buf).unwrap_err();
        matches!(err, Error::Other(_));
    }

    #[test]
    fn test_to_be_bytes_sending_frame() {
        // Example: RegisterResult with StatusCodes::SUCCESS
        let operation = SendingOperations::RegisterResult;
        let frame = SendingFrames::RegisterResult(StatusCodes::SUCCESS);
        let bytes = frame.frame_to_be_bytes();
        // Should start with operation byte, then frame bytes
        assert_eq!(bytes[0], operation as u8);
        assert_eq!(bytes[1], StatusCodes::SUCCESS.into_u8());

        // Example: PingResult with HealthStatus = 1 (assuming HealthStatus is u8)
        let operation = SendingOperations::PingResult;
        let frame = SendingFrames::PingResult(HealthStatus::HEALTHY);
        let bytes = frame.frame_to_be_bytes();
        assert_eq!(bytes[0], operation as u8);
        assert_eq!(bytes[1], HealthStatus::HEALTHY as u8);

        // Example: Command with PeerCommands::SYNC
        let operation = SendingOperations::Commands;
        let frame = SendingFrames::Command(PeerCommands::SYNC);
        let bytes = frame.frame_to_be_bytes();
        assert_eq!(bytes[0], operation as u8);
        assert_eq!(bytes[1], SYNC_CODE);

        // Example: DoConnectResult with StatusCodes::FAILURE
        let operation = SendingOperations::DoConnectResult;
        let frame = SendingFrames::DoConnectResult(StatusCodes::FAILURE);
        let bytes = frame.frame_to_be_bytes();
        assert_eq!(bytes[0], operation as u8);
        assert_eq!(bytes[1], StatusCodes::FAILURE.into_u8());

        // Example: ProbeResult with StatusCodes::SUCCESS and None PeerInfo
        let operation = SendingOperations::ProbeResult;
        let frame = SendingFrames::ProbeResult(None);
        let bytes = frame.frame_to_be_bytes();
        assert_eq!(bytes[0], operation as u8);
        assert_eq!(bytes[1], StatusCodes::FAILURE.into_u8());
        assert_eq!(bytes.len(), 2);

        // Example: ProbeResult with StatusCodes::SUCCESS and Some PeerInfo
        // You may need to construct a valid PeerInfo for your implementation
        // Here we assume PeerInfo::default() exists for demonstration
        let dummy_peer_info = PeerInfo::default(); // Replace with actual PeerInfo if needed
        let operation = SendingOperations::ProbeResult;
        let frame = SendingFrames::ProbeResult((Some(dummy_peer_info.clone())));
        let bytes = frame.frame_to_be_bytes();
        assert_eq!(bytes[0], operation as u8);
        assert_eq!(bytes[1], StatusCodes::SUCCESS.into_u8());
        let expected_peer_info_bytes = dummy_peer_info.to_be_bytes();
        assert_eq!(&bytes[2..], &expected_peer_info_bytes[..]);
    }
}
