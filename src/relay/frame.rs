use std::{fmt::Display, io::Cursor, net::IpAddr};

use bytes::Buf;

use crate::relay::relay::{PeerId, PeerInfo, PEER_ID_LENGTH_BYTES};

// Error for Frame operation
#[derive(Debug)]
pub enum Error {
    // Cannot read the wholr Frame. see [Frame] below
    Incomplete,
    Other(std::io::Error),
}

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

/// API operation
/// Server accepts only requests starting with these operations.
/// Usually every request can be described as
/// |operation| body  ...
/// | 1 byte  | n|0 bytes ...
///
/// These operations need to be run in series to make connection succesfully between peers
pub enum ReceivingOperations {
    // 01
    // Register peer with server, so that other peers can reach it.
    Register = 0x01,
    // 02
    // Probe the peer and get details to connect to it
    Probe = 0x02,
    // 03
    // Tell peer to initiate connection. See [RelayServer].
    DoConnect = 0x03,
    // 04
    // Ping Relay Server to get Round Trip Time
    Ping = 0x04,
}

impl TryFrom<u8> for ReceivingOperations {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(ReceivingOperations::Register),
            0x02 => Ok(ReceivingOperations::Probe),
            0x03 => Ok(ReceivingOperations::DoConnect),
            0x04 => Ok(ReceivingOperations::Ping),
            _ => Err(Error::Other(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Operation not supported",
            ))),
        }
    }
}

/// Operations used by Relay Server to send data to peers, eg. responses
pub enum SendingOperations {
    // ========================== SOURCE PEER ==========================
    // Response status we send to source peer
    // 21
    // Register peer with server, so that other peers can reach it.
    RegisterResult = 0x21,
    // 22
    // Send Destination PeerInfo to source peer
    ProbeResult = 0x22,
    // 23
    // Send synchronization result to source peer
    DoConnectResult = 0x23,
    // 24
    // Send Ping reult
    PingResult = 0x24,
    // ========================== DESTINATION PEER ==========================
    // 25
    // Send commands to destination peer to fetch rtt or telling it to start hole punching request
    Commands = 0x25,
}

impl TryFrom<u8> for SendingOperations {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x21 => Ok(SendingOperations::RegisterResult),
            0x22 => Ok(SendingOperations::ProbeResult),
            0x23 => Ok(SendingOperations::DoConnectResult),
            0x24 => Ok(SendingOperations::PingResult),
            0x25 => Ok(SendingOperations::Commands),
            _ => Err(Error::Other(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Sending Operation not supported",
            ))),
        }
    }
}

#[derive(Clone)]
pub enum HealthStatus {
    HEALTHY = 0x01,
    UNHEALTHY = 0x02,
}

impl TryFrom<u8> for HealthStatus {
    type Error = Error;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(HealthStatus::HEALTHY),
            0x02 => Ok(HealthStatus::UNHEALTHY),
            _ => Err(Error::Other(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Sending Operation not supported",
            ))),
        }
    }
}

pub trait Frame<O, F, E>
where
    O: TryFrom<u8>,
{
    // Reads operation from buf, and maps it to Operations allowed for that Frame
    fn read_operation(buf: &mut Cursor<&[u8]>) -> Result<O, E>;

    // Reads buf, and maps it to Frame of type F
    fn parse(buf: &mut Cursor<&[u8]>, operation: O) -> Result<F, E>;

    // Writes operation and then Frame payload into a vector of bytes
    fn to_be_bytes_prepend_operation(operation: O, frame: F) -> Vec<u8>;

    // Frame to big endian bytes
    fn to_be_bytes(self) -> Vec<u8>;
}

// Payload for frame received by Relay Server usually requests from peers except ProbeAck which is Response from destination peer
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
    fn parse(
        buf: &mut Cursor<&[u8]>,
        operation: ReceivingOperations,
    ) -> Result<RecevingFrames, Error> {
        match operation {
            ReceivingOperations::Register => Ok(RecevingFrames::Register(PeerId::try_from(buf)?)),
            ReceivingOperations::Probe => Ok(RecevingFrames::Register(PeerId::try_from(buf)?)),
            ReceivingOperations::DoConnect => Ok(RecevingFrames::Register(PeerId::try_from(buf)?)),
            ReceivingOperations::Ping => Ok(RecevingFrames::Ping),
        }
    }

    fn to_be_bytes_prepend_operation(
        operation: ReceivingOperations,
        frame: RecevingFrames,
    ) -> Vec<u8> {
        let mut bytes: Vec<u8> = Vec::with_capacity(1024);
        bytes.push(operation as u8);
        bytes.extend_from_slice(&frame.to_be_bytes());
        bytes
    }

    fn to_be_bytes(self) -> Vec<u8> {
        todo!()
        //     match self {
        //         RecevingFrames::Register(peer_id) => {
        //             let frame = vec![]
        //             Ok(vec![].extend_from_slice(peer_id.0),),
        //         }
        //         RecevingFrames::Probe(peer_id) => todo!(),
        //         RecevingFrames::DoConnect(peer_id) => todo!(),
        //         RecevingFrames::Ping => todo!(),
        //         RecevingFrames::ProbeAck => todo!(),
        //     }
    }
}

/// Frames sent to peers, eg. Response frames and Commands to destination peers
pub enum SendingFrames {
    RegisterResult(StatusCodes),
    ProbeResult((StatusCodes, Option<PeerInfo>)),
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
    fn parse(
        buf: &mut Cursor<&[u8]>,
        operation: SendingOperations,
    ) -> Result<SendingFrames, Error> {
        match operation {
            SendingOperations::RegisterResult => todo!(),
            SendingOperations::ProbeResult => todo!(),
            SendingOperations::DoConnectResult => todo!(),
            SendingOperations::PingResult => todo!(),
            SendingOperations::Commands => todo!(),
        }
    }

    fn to_be_bytes_prepend_operation(
        operation: SendingOperations,
        frame: SendingFrames,
    ) -> Vec<u8> {
        let mut bytes = vec![operation as u8];
        bytes.extend_from_slice(&frame.to_be_bytes());
        bytes
    }

    fn to_be_bytes(self) -> Vec<u8> {
        match self {
            SendingFrames::RegisterResult(status_codes) => {
                // FIX ME
                vec![status_codes.clone().into_u8()]
            }
            SendingFrames::ProbeResult(peer_info) => {
                let sc = peer_info.0.clone();
                let pi = peer_info.1.clone();
                let mut bytes = vec![sc.clone().into_u8()];
                if let Some(pi) = pi {
                    bytes.extend(pi.to_be_bytes());
                }
                return bytes;
            }
            SendingFrames::Command(peer_commands) => peer_commands.to_be_bytes(),
            SendingFrames::DoConnectResult(status_codes) => {
                vec![status_codes.clone().into_u8()]
            }
            SendingFrames::PingResult(health_status) => vec![health_status as u8],
        }
    }
}

const SYNC_CODE: u8 = 0x91;
const DOCONNECT_CODE: u8 = 0x92;

#[derive(Clone)]
pub enum StatusCodes {
    SUCCESS = 0x100,
    FAILURE = 0x101,
}

impl StatusCodes {
    pub fn into_u8(self) -> u8 {
        match self {
            StatusCodes::SUCCESS => StatusCodes::SUCCESS as u8,
            StatusCodes::FAILURE => StatusCodes::FAILURE as u8,
        }
    }
}

// FIX ME
#[derive(Clone)]
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

#[cfg(test)]
mod Tests {}
