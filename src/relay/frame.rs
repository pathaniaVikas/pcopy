use std::{fmt::Display, io::Cursor, net::IpAddr};

use bytes::Buf;

use crate::relay::relay::{PeerId, PeerInfo, PEER_ID_LENGTH_BYTES};

/// API operation
/// Server accepts only requests starting with these operations.
/// Usually every request can be described as
/// |operation| body  ...
/// | 1 byte  | n|0 bytes ...
///
/// These operations need to be run in series to make connection succesfully between peers
pub enum Operation {
    // 01
    // Register peer with server, so that other peers can reach it.
    Register = 0x01,
    // 02
    // Probe the peer and get details to connect to it
    Probe = 0x02,
    // 03
    // Tell peer to initiate connection. See [RelayServer].
    Synchronize = 0x03,
    // 04
    // Ping Relay Server to get Round Trip Time
    Ping = 0x04,
}

impl TryFrom<u8> for Operation {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Operation::Register),
            0x02 => Ok(Operation::Probe),
            0x03 => Ok(Operation::Synchronize),
            0x04 => Ok(Operation::Ping),
            _ => Err(Error::Other(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Operation not supported",
            ))),
        }
    }
}

#[derive(Debug)]
pub enum Error {
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

// impl<T: std::error::Error + Send + Sync + 'static> From<T> for Error<T> {
//     fn from(e: T) -> Self {
//         Error::Other(e)
//     }
// }

pub enum RecevingFrames {
    Register(PeerId),
    Probe(PeerId),
    Synchronize(PeerId),
    Ping,
    ProbeAck,
}

impl RecevingFrames {
    /// Advances cursor by OPERATION_LENGTH bytes
    pub fn read_operation(buf: &mut Cursor<&[u8]>) -> Result<Operation, Error> {
        if !buf.has_remaining() {
            return Err(Error::Incomplete);
        }

        // Cursor is advanced by 1 byte here
        Ok(Operation::try_from(buf.get_u8())?)
    }

    /// Advances cusror by Frame length
    pub fn parse(buf: &mut Cursor<&[u8]>, operation: Operation) -> Result<RecevingFrames, Error> {
        match operation {
            Operation::Register => Ok(RecevingFrames::Register(PeerId::try_from(buf)?)),
            Operation::Probe => Ok(RecevingFrames::Register(PeerId::try_from(buf)?)),
            Operation::Synchronize => Ok(RecevingFrames::Register(PeerId::try_from(buf)?)),
            Operation::Ping => Ok(RecevingFrames::Ping),
        }
    }
}

pub enum SendingFrames {
    Registered(StatusCodes),
    ProbeResult((StatusCodes, Option<PeerInfo>)),
    SynchronizeResult(StatusCodes),
    PingResult,
    Command(PeerCommands),
}

impl SendingFrames {
    pub fn to_be_bytes(self) -> Vec<u8> {
        match self {
            SendingFrames::Registered(status_codes) => {
                // FIX ME
                vec![125_u8, status_codes.clone().into_u8()]
            }
            SendingFrames::ProbeResult(peer_info) => {
                let sc = peer_info.0.clone();
                let pi = peer_info.1.clone();
                let mut bytes = vec![126_u8, sc.clone().into_u8()];
                if let Some(pi) = pi {
                    bytes.extend(pi.to_be_bytes());
                }
                return bytes;
            }
            SendingFrames::Command(peer_commands) => peer_commands.to_be_bytes(),
            SendingFrames::SynchronizeResult(status_codes) => {
                vec![127_u8, status_codes.clone().into_u8()]
            }
            SendingFrames::PingResult => vec![201_u8],
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
