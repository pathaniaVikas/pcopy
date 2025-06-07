use std::{
    collections::HashMap,
    fmt::Display,
    io::{Error, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{mpsc::SendError, Arc},
    thread,
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    time::Instant,
};
use tracing::{error, info};

pub enum ResponseCodes {
    RegisterSuccess,
    RegisterFailure,
    ConnectSuccess,
    ConnectFailure,
    // InitiateConnectSuccess,
    InitiateConnectFailure,
}

impl TryInto<u32> for ResponseCodes {
    type Error = Error;

    fn try_into(self) -> Result<u32, Self::Error> {
        match self {
            ResponseCodes::RegisterSuccess => Ok(0x01_u32),
            ResponseCodes::RegisterFailure => Ok(0x02_u32),
            ResponseCodes::ConnectSuccess => Ok(0x03_u32),
            ResponseCodes::ConnectFailure => Ok(0x04_u32),
            ResponseCodes::InitiateConnectFailure => Ok(0x05_u32),
            _ => Err(Error::new(
                std::io::ErrorKind::Unsupported,
                "ResponseCode does not exist",
            )),
        }
    }
}

pub enum PeerRequestCodes {
    Ping,
    InitConnect,
}

impl TryInto<u32> for PeerRequestCodes {
    type Error = Error;

    fn try_into(self) -> Result<u32, Self::Error> {
        match self {
            PeerRequestCodes::Ping => Ok(0x11_u32),
            PeerRequestCodes::InitConnect => Ok(0x12_u32),
        }
    }
}

/// Represents peer metadata
/// Ip: Peer public IP
/// ping_rtt: Round trip time it takes to reach to peer from Relay server
#[derive(Debug, Clone)]
pub struct PeerInfo {
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

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct PeerId(Vec<u8>);

impl Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match String::from_utf8(self.0.clone()) {
            Ok(peer_id_string) => {
                write!(f, "{}", peer_id_string)?;
            }
            Err(_) => {
                write!(f, "{}", "MalformedPeerId")?;
            }
        }
        Ok(())
    }
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
///
///

///
pub trait RelayServer {
    /// Register: Register the peer with server, saving its IpAddr and Socket connection
    ///           See [ConnectionMetadata]
    fn register(
        &mut self,
        peer_id: PeerId,
        source_ip: IpAddr,
        socket: Arc<Mutex<TcpStream>>,
    ) -> Result<bool, Error>;

    /// Probe: Probe the peer and find its details like its IP, and Time it takes to reach peer from Relay server
    ///        Returns [PeerInfo]
    ///        This gives the asking peer, the idea of total_RTT from "source_peer -> RelayServer -> destination_peer"
    ///
    async fn probe(&mut self, peer_id: PeerId) -> Result<PeerInfo, Error>;

    /// Synchronize: Source peer tells destination peer to start the connection for hole punching to succeed.
    ///         As peer receives this request, it opens connection to source peer.
    ///         Source Peer waits for total_rtt//2 and initiates connection to destination peer.
    ///         Once both connects, both sends Ack to Relay server.
    ///         Relay server closes both the connections
    async fn synchronize(&mut self, destination_peer_id: PeerId) -> Result<(), Error>;

    /// Just sends empty response, to be used to probe connection time between peer and relay server
    fn ping(&mut self) -> Result<bool, Error>;
}

/// Details about peer connection.
/// tcp_stream: TCP socket (may be closed after two peers are connected)
/// ip_addr: Public Ip Address of peer, where it can be reached.
#[derive(Clone)]
struct PeerConnectionMetadata {
    tcp_stream: Arc<Mutex<TcpStream>>,
    ip_addr: IpAddr,
}

/// Main logic to listen to connections and process them.
/// Server maintains map of [PEER_ID] -> [PeerConnectionMetadata]
/// to be used for hole punching. see [RelayServer]
/// TODO: Currently we don't delete items from the map, needs to be fixed
struct Server {
    server_address: String,
    conn_map: HashMap<PeerId, PeerConnectionMetadata>,
}

impl Server {
    /// get server "Ip,Port"
    pub fn get_server_address(&self) -> String {
        self.server_address.clone()
    }

    /// Given peer id, returns the socket registered for that peer
    /// Do not guarantee that socket is alive
    pub fn get_connection(&self, peer_id: &PeerId) -> Option<PeerConnectionMetadata> {
        match self.conn_map.get(peer_id) {
            Some(conn_md) => Some(conn_md.clone()),
            None => None,
        }
    }
}

impl RelayServer for Server {
    fn register(
        &mut self,
        peer_id: PeerId,
        source_ip: IpAddr,
        socket: Arc<Mutex<TcpStream>>,
    ) -> Result<bool, Error> {
        let conn_md = PeerConnectionMetadata {
            tcp_stream: socket,
            ip_addr: source_ip,
        };

        self.conn_map.insert(peer_id, conn_md);

        Ok(true)
    }

    async fn probe(&mut self, peer_id: PeerId) -> Result<PeerInfo, Error> {
        let ping = (PeerRequestCodes::Ping as u32).to_be_bytes();
        if let Some(conn_md) = self.get_connection(&peer_id) {
            let mut max_tries = 3;
            let mut time_to_reach_peer = Duration::default();

            while max_tries > 0 {
                let start = Instant::now();

                match conn_md
                    .tcp_stream
                    .try_lock()
                    .unwrap()
                    .write_all(&ping)
                    .await
                {
                    Ok(_) => {
                        // Read one byte client response
                        let read_until = 1;
                        let mut n = 0;
                        let mut buf = vec![0; 1];

                        while n <= read_until {
                            n = conn_md
                                .tcp_stream
                                .try_lock()
                                .unwrap()
                                .read(&mut buf)
                                .await
                                .expect("failed to read data from socket");
                        }
                        // Once we got the response from peer, send the time
                        // it takes for a tcp ping request to asking client
                        time_to_reach_peer = start.elapsed();
                        let peer_info = PeerInfo {
                            ip: conn_md.ip_addr,
                            ping_rtt: time_to_reach_peer,
                        };
                        return Ok(peer_info);
                    }
                    Err(e) => {
                        error!(
                            "{}",
                            format!(
                                "Cannot relay connection to socket {}. Error {}. \n Retry Left {}",
                                conn_md.ip_addr, e, max_tries
                            )
                        );

                        thread::sleep(Duration::from_secs(1));
                        max_tries -= 1;
                    }
                };
            }
        } else {
            error!(
                "{}",
                format!(
                    "Cannot relay connection to socket. Peer {} not registered yet",
                    String::from_utf8(peer_id.0).unwrap()
                )
            );
            return Err(Error::new(
                std::io::ErrorKind::NotFound,
                "Peer not found in cache",
            ));
        }

        return Err(Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            format!("Cannot connect to peer {}", peer_id),
        ));
    }

    async fn synchronize(&mut self, destination_peer_id: PeerId) -> Result<(), Error> {
        if let Some(conn_md) = self.get_connection(&destination_peer_id) {
            let init_connect_request = create_synchronize_request(&conn_md);

            match conn_md
                .tcp_stream
                .try_lock()
                .unwrap()
                .write_all(&init_connect_request)
                .await
            {
                Ok(_) => {
                    // Read one byte client response, sent when direct conn is established
                    // between two peers
                    let read_until = 1;
                    let mut n = 0;
                    let mut buf_new = vec![0; 1];

                    while n <= read_until {
                        n = conn_md
                            .tcp_stream
                            .try_lock()
                            .unwrap()
                            .read(&mut buf_new)
                            .await
                            .expect("failed to read data from socket");
                    }

                    info!(
                        "{}",
                        format!(
                            "Peer has made connection sucesfully to peer with ip {}",
                            conn_md.ip_addr.to_string()
                        )
                    );

                    // TODO: Shutdown connection
                    // co.tcp_listener.try_lock().unwrap().shutdown().await;
                }
                Err(e) => {
                    error!(
                        "{}",
                        format!(
                            "Cannot relay synchronize connection request to peer {}. Error {}.",
                            conn_md.ip_addr.to_string(),
                            e
                        )
                    );
                    return Err(Error::new(
                        std::io::ErrorKind::AddrNotAvailable,
                        "Cant Send connection init request to peer",
                    ));
                }
            };
        } else {
            error!(
                "{}",
                format!(
                    "Cannot send synchronize request to peer. Peer {} not registered yet",
                    String::from_utf8(destination_peer_id.0).unwrap()
                )
            );
            return Err(Error::new(
                std::io::ErrorKind::NotFound,
                "Peer not found in cache",
            ));
        }

        Ok(())
    }

    fn ping(&mut self) -> Result<bool, Error> {
        todo!()
    }
}

/// We need a mutable handle to Server, since it holds a hashmap of <[PEER_ID], <[PeerConnectionMetadata]>.
/// This provides a mutex over the server, so that multiple connections/tokio tasks can access
/// the internal hashmap
#[derive(Clone)]
struct MutableServerHandle {
    inner: Arc<Mutex<Server>>,
}

impl MutableServerHandle {
    pub fn new(addr: String) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Server {
                server_address: addr,
                conn_map: HashMap::new(),
            })),
        }
    }

    /// Exposed method, to call methods on server with lock
    pub fn with_lock<F, T>(&self, func: F) -> T
    where
        F: FnOnce(&mut Server) -> T,
    {
        let mut lock = self.inner.try_lock().unwrap();
        let result = func(&mut *lock);
        drop(lock);
        result
    }

    /// Mutex version of register peer call
    pub fn register<'a>(
        &self,
        peer_id: PeerId,
        source_ip: IpAddr,
        socket: Arc<Mutex<TcpStream>>,
    ) -> Result<bool, Error> {
        self.with_lock(|server| server.register(peer_id, source_ip, socket))
    }

    /// Mutex version of probe peer call
    /// TODO: Do we need lock here ?
    pub async fn probe<'a>(&self, peer_id: PeerId) -> Result<PeerInfo, Error> {
        self.inner.try_lock().unwrap().probe(peer_id).await
    }

    /// Mutex version of probe peer call
    /// TODO: Do we need lock here ?
    pub async fn synchronize<'a>(&self, destination_peer_id: PeerId) -> Result<(), Error> {
        self.inner
            .try_lock()
            .unwrap()
            .synchronize(destination_peer_id)
            .await
    }

    /// Main server loop
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let server_address = self.with_lock(|server| server.get_server_address());

        let listener = TcpListener::bind(server_address.clone()).await?;

        info!("Server started on ip: {}", server_address);

        loop {
            let (socket, sock_addr) = listener.accept().await?;
            let cloned_self = self.clone();
            tokio::spawn(async move {
                cloned_self.process_connection(socket, sock_addr).await;
            });
        }
    }

    async fn process_connection(self, mut socket: TcpStream, sock_addr: SocketAddr) {
        let mut buf = vec![0; 1024];
        let mut n = 0;
        // First Byte | Operation |
        let mut read_until = 1;
        while n <= read_until {
            // Keep the connection open
            n = socket
                .read(&mut buf)
                .await
                .expect("failed to read data from socket");
        }

        if buf[0] == Operation::Register as u8 {
            // Peer Id is 128 bytes
            read_until = read_until + 128;

            read_from_socket(&mut n, read_until, &mut socket, &mut buf).await;

            let socket_to_copy = Arc::new(Mutex::new(socket));
            let peer_id: Vec<_> = buf[1..129].iter().cloned().collect();

            match self.register(
                PeerId(peer_id.clone()),
                sock_addr.ip(),
                socket_to_copy.clone(),
            ) {
                Ok(_) => {
                    info!(
                        "Peer registered with peer id {}",
                        String::from_utf8(peer_id).unwrap()
                    );
                    let response = (ResponseCodes::RegisterSuccess as u32).to_be_bytes();
                    socket_to_copy
                        .try_lock()
                        .unwrap()
                        .write_all(&response)
                        .await;
                }
                Err(e) => error!("Peer failed to register {}", e),
            };
        } else if buf[0] == Operation::Probe as u8 {
            // Peer Id is 128 bytes
            read_until = read_until + 128;
            read_from_socket(&mut n, read_until, &mut socket, &mut buf).await;
            let peer_id = PeerId(buf[1..129].iter().cloned().collect());
            match self.probe(peer_id).await {
                Ok(peer_info) => {
                    socket.write_all(&peer_info.to_be_bytes()).await;
                }
                Err(_) => {
                    socket
                        .write_all(&(ResponseCodes::ConnectFailure as u32).to_be_bytes())
                        .await;
                }
            }
        } else if buf[0] == Operation::Synchronize as u8 {
            // Peer Id is 128 bytes
            read_until = read_until + 128;
            read_from_socket(&mut n, read_until, &mut socket, &mut buf).await;
            let peer_id = PeerId(buf[1..129].iter().cloned().collect());

            // This will shutdown both the connections from two peers
            match self.synchronize(peer_id).await {
                Ok(_) => {
                    // Once we got the response from one peer, check response from peer
                    // which started the connection
                    read_until = read_until + 1;
                    read_from_socket(&mut n, read_until, &mut socket, &mut buf).await;

                    info!(
                        "{}",
                        format!(
                            "Peer 1 with ip {} has made connection sucesfully to Peer 2",
                            sock_addr
                        )
                    );
                }
                Err(_) => {
                    error!("Cannot Initiate Connection with peer");
                    socket
                        .write_all(&(ResponseCodes::InitiateConnectFailure as u32).to_be_bytes())
                        .await;
                }
            }

            // TODO: Shutdown connection
            // socket.shutdown();
        }
    }
}

async fn read_from_socket(
    n: &mut usize,
    read_until: usize,
    socket: &mut TcpStream,
    buf: &mut Vec<u8>,
) {
    while *n <= read_until {
        *n = socket
            .read(buf)
            .await
            .expect("failed to read data from socket");
    }
}

fn create_synchronize_request(conn_md: &PeerConnectionMetadata) -> Vec<u8> {
    let mut init_connect_request = Vec::new();
    init_connect_request.extend_from_slice(&(PeerRequestCodes::InitConnect as u32).to_be_bytes());
    let ip_bytes = match conn_md.ip_addr {
        IpAddr::V4(ip) => ip.octets().to_vec(),
        IpAddr::V6(ip) => ip.octets().to_vec(),
    };
    init_connect_request.extend_from_slice(&ip_bytes);
    init_connect_request
}
