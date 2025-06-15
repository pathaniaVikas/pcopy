use std::{
    collections::HashMap,
    fmt::Display,
    io::{Cursor, Error, Read, Seek},
    net::{IpAddr, SocketAddr},
    sync::Arc,
    thread,
    time::Duration,
};

use bytes::{Buf, BytesMut};
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt, BufWriter},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    time::Instant,
};
use tracing::{error, info, warn};

use crate::relay::{
    cache::Cache,
    connection::Connection,
    frame::{PeerCommands, RecevingFrames, StatusCodes},
};

pub const PEER_ID_LENGTH_BYTES: usize = 64;
pub const PROBE_ACK_LENGTH: usize = 1;

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
    peer_id: PeerId,
    ip: IpAddr,
    ping_rtt: Duration,
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
struct PeerConnectionMetadata {
    stream: Arc<Mutex<BufWriter<TcpStream>>>,
    ip_addr: IpAddr,
}

/// Relay Server is used to help two peers connect to each other.
/// We are trying to mimic algorithm for TCP hole punching.
///
/// LibP2p has implemenation for hole punching here: https://docs.rs/libp2p/latest/libp2p/tutorials/hole_punching/index.html
/// which is used as guide for this code.
/// Code here is highly simplified and just me trying to implement the idea of hole punching.
///
/// Basically client needs following information at least to hole puch through other peers
/// PeerIp: Peer public IP
/// PeerRtt: Round trip time it
///
/// We also need one more information so that peers can identify each other without knowing their IP
/// PeerId: Unique id given to every peer in network.
///         This is used to identify peer without knowing its IP address.
///
/// ```ignore
/// Now lets say we have 2 peers (Tom and Jerry)
///                                                Time                                            
///                                                 |                                              
///                                                 |                                              
///                    Peer 1 Tom                   |             Peer 2 Jerry                     
///                                                 |                                              
///                                                 |                                              
///                                                 |                                              
///                                                 |                                              
///            1. Register with relay server        |      1. Register with relay server           
///                                                 |                                              
///                     Tom :<ip_addr, socket>      |           Jerry  :<ip_addr, socket>          
///                                                 |                                              
///            2. Ping Relay server                 |                                              
///                                                 |                                              
///            2. Probe Jerry                       |      2. Wait for Probe call from Relay server
///                                                 |           Respond                            
///                     Jerry:                      |                                              
///                           source_ip             |                                              
///                           rtt                   |                                              
///                                                 |                                              
///                     total_rtte = probe_time+rtt |                                              
///                                                 |                                              
///            3. Synchronize with Jerry            |       3. Wait for Synchronize call           
///                                                 |                                              
///                  After total_time/2:            |                                              
///                     send conn request to Jerry  |             Connect to Tom                   
///                                                 |                                                                    
///                                                 |                                              
///                                                 |                                              
///                                                 v          
///
/// This trait defines methods for client actions described above. see [Operation]
/// ```
///

///
pub trait RelayServer {
    /// Register: Register the peer with server, saving its IpAddr and Socket connection
    ///           See [ConnectionMetadata]
    async fn register(
        &self,
        peer_id: PeerId,
        source_ip: IpAddr,
        connection: Arc<Mutex<BufWriter<TcpStream>>>,
    ) -> Result<(), Error>;

    /// Probe: Probe the peer and find its details like its IP, and Time it takes to reach peer from Relay server
    ///        Returns [PeerInfo]
    ///        This gives the asking peer, the idea of total_RTT from "source_peer -> RelayServer -> destination_peer"
    ///
    async fn probe(&self, peer_id: PeerId) -> Result<PeerInfo, Error>;

    /// Synchronize: Source peer tells destination peer to start the connection for hole punching to succeed.
    ///         As peer receives this request, it opens connection to source peer.
    ///         Source Peer waits for total_rtt//2 and initiates connection to destination peer.
    ///         Once both connects, both sends Ack to Relay server.
    ///         Relay server closes both the connections
    async fn synchronize(
        &self,
        destination_peer_id: PeerId,
        source_ip: IpAddr,
    ) -> Result<(), Error>;

    /// Just sends empty response, to be used to probe connection time between peer and relay server
    fn ping(&mut self) -> Result<bool, Error>;
}

/// Main logic to listen to connections and process them.
/// Server maintains map of [PEER_ID] -> [PeerConnectionMetadata]
/// to be used for hole punching. see [RelayServer]
/// TODO: Currently we don't delete items from the map, needs to be fixed

#[derive(Clone)]
struct Server {
    pub server_address: String,
    pub conn_cache: Arc<Cache<PeerId, PeerConnectionMetadata>>,
}

impl Server {
    pub fn new(ip: IpAddr, port: u32) -> Self {
        Server {
            server_address: format!("{}:{port}", ip.to_string()),
            conn_cache: Arc::new(Cache::new(1024)),
        }
    }

    /// get server "Ip,Port"
    pub fn get_server_address(&self) -> String {
        self.server_address.clone()
    }
}

impl RelayServer for Server {
    async fn register(
        &self,
        peer_id: PeerId,
        source_ip: IpAddr,
        stream: Arc<Mutex<BufWriter<TcpStream>>>,
    ) -> Result<(), Error> {
        let conn_md = PeerConnectionMetadata {
            stream: stream.clone(),
            ip_addr: source_ip,
        };
        self.conn_cache.insert(peer_id, conn_md).unwrap();
        Ok(())
    }

    ///
    /// 1. Send ping request to peer
    /// 2. Wait for its response
    /// 3. If error connecti
    async fn probe(&self, peer_id: PeerId) -> Result<PeerInfo, Error> {
        if let Some(conn_md) = self.conn_cache.get(&peer_id) {
            let start = Instant::now();
            Connection::write_frame_to_stream(
                conn_md.stream.clone(),
                super::frame::SendingFrames::Command(PeerCommands::SYNC),
            )
            .await?;
            let mut buffer = BytesMut::with_capacity(PROBE_ACK_LENGTH);
            match Connection::read_frame_from_stream(conn_md.stream.clone(), &mut buffer)
                .await
                .unwrap()
            {
                Some(frame) => match frame {
                    RecevingFrames::ProbeAck => {
                        let time_to_reach_peer = start.elapsed();
                        let peer_info = PeerInfo {
                            peer_id: peer_id.clone(),
                            ip: conn_md.ip_addr,
                            ping_rtt: time_to_reach_peer,
                        };
                        return Ok(peer_info);
                    }
                    _ => {
                        // This sitution should never arrive, it might mean some other
                        // shared task, has written to the same
                        // TODO: Although buffers are not shared, stream is shared, so it may be
                        // possible that another task, wrote its own Frame in the stream.
                        // Eg. while peer A is probing and sent SYNC command to peer B
                        // peer B may call register again and now that other task, writes RegisterSuccess to the stream
                        // Fix this by taking lock over critical sections
                        return Err(Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Invalid data received from peer {}", peer_id),
                        ));
                    }
                },
                None => {
                    return Err(Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        format!("Ack not received from peer {}", peer_id),
                    ))
                }
            }
        } else {
            return Err(Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Peer {} not registered yet", peer_id),
            ));
        }
    }

    async fn synchronize(
        &self,
        destination_peer_id: PeerId,
        source_ip: IpAddr,
    ) -> Result<(), Error> {
        if let Some(conn_md) = self.conn_cache.get(&destination_peer_id) {
            Connection::write_frame_to_stream(
                conn_md.stream.clone(),
                super::frame::SendingFrames::Command(PeerCommands::DOCONNECT(source_ip)),
            )
            .await?;
        } else {
            return Err(Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Peer {} not registered yet", destination_peer_id),
            ));
        }

        Ok(())
    }

    fn ping(&mut self) -> Result<bool, Error> {
        todo!()
    }
}

#[derive(Clone)]
struct ShareableServerHandle {
    inner: Arc<Server>,
}

impl ShareableServerHandle {
    pub fn new(ip: IpAddr, port: u32) -> Self {
        ShareableServerHandle {
            inner: Arc::new(Server::new(ip, port)),
        }
    }

    /// Main server loop
    /// SET KEEP ALIVE ON CONNECTION
    /// SET CONN TIMEOUT
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let server_address = self.inner.get_server_address();
        let listener = TcpListener::bind(server_address.clone()).await?;

        info!("Server started on ip: {}", server_address);

        loop {
            let (socket, peer_addr) = listener.accept().await?;
            // let cloned_self = self.clone();
            let connection = Connection::new(Arc::new(Mutex::new(BufWriter::new(socket))));
            let server = self.inner.clone();
            tokio::spawn(async move {
                let _ = process_connection(server, connection, peer_addr).await;
            });
        }
    }
}

/// What if
/// client A => register()  |
/// write_resp1 to cleint A |
///                         |client B => register()
/// client A => Probes      |
/// write_resp2 to client B | Write resp1 to client B
///
async fn process_connection(
    server: Arc<Server>,
    mut connection: Connection,
    peer_address: SocketAddr,
) -> io::Result<()> {
    loop {
        match connection.read_frame().await.unwrap() {
            Some(frame) => match frame {
                RecevingFrames::Register(peer_id) => {
                    // let lock = connection.stream.get_ref().
                    match server
                        .register(
                            peer_id.clone(),
                            peer_address.ip(),
                            connection.stream.clone(),
                        )
                        .await
                    {
                        Ok(_) => {
                            connection
                                .write_frame(super::frame::SendingFrames::Registered(
                                    StatusCodes::SUCCESS,
                                ))
                                .await?;
                        }
                        Err(_) => {
                            connection
                                .write_frame(super::frame::SendingFrames::Registered(
                                    StatusCodes::FAILURE,
                                ))
                                .await?;
                        }
                    }
                }
                RecevingFrames::Probe(peer_id) => match server.probe(peer_id).await {
                    Ok(peer_info) => {
                        connection
                            .write_frame(super::frame::SendingFrames::ProbeResult((
                                StatusCodes::SUCCESS,
                                Some(peer_info),
                            )))
                            .await?;
                    }
                    Err(_) => {
                        connection
                            .write_frame(super::frame::SendingFrames::ProbeResult((
                                StatusCodes::FAILURE,
                                None,
                            )))
                            .await?;
                    }
                },
                RecevingFrames::Synchronize(peer_id) => {
                    match server.synchronize(peer_id, peer_address.ip()).await {
                        Ok(()) => {
                            connection
                                .write_frame(super::frame::SendingFrames::SynchronizeResult(
                                    StatusCodes::SUCCESS,
                                ))
                                .await?;
                        }
                        Err(_) => {
                            connection
                                .write_frame(super::frame::SendingFrames::SynchronizeResult(
                                    StatusCodes::FAILURE,
                                ))
                                .await?;
                        }
                    }
                }
                RecevingFrames::Ping => {
                    connection
                        .write_frame(super::frame::SendingFrames::PingResult)
                        .await?;
                }
                RecevingFrames::ProbeAck => {
                    // Proble Ack is supposed to be read by Probe method, not here.
                    // This indicates some invalid state
                    error!("Unexpected ProbeAck found");
                }
            },
            None => return Ok(()),
        }
    }
}
