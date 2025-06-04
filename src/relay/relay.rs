use std::{
    collections::HashMap,
    io::{Error, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{mpsc::SendError, Arc},
    thread,
    time::Duration,
};

use iced::futures::TryFutureExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    time::Instant,
};
use tracing::{error, info};

type PEER_ID = Vec<u8>;
enum ResponseCodes {
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
        }
    }
}

enum RequestCodes {
    Ping,
    InitConnect,
}

impl TryInto<u32> for RequestCodes {
    type Error = Error;

    fn try_into(self) -> Result<u32, Self::Error> {
        match self {
            RequestCodes::Ping => Ok(0x11_u32),
            RequestCodes::InitConnect => Ok(0x12_u32),
        }
    }
}

trait RelayServer {
    fn register(
        &mut self,
        peer_id: PEER_ID,
        source_ip: IpAddr,
        socket: Arc<Mutex<TcpStream>>,
    ) -> Result<bool, Error>;

    fn connect(&mut self, target_ip: IpAddr, source_ip: IpAddr) -> Result<u128, Error>;
}

enum Operation {
    // 01
    Register = 0x01,
    // 02
    Relay = 0x02,
    // 03
    InitiateConnect = 0x03,
}

#[derive(Clone)]
struct ConnectionMetadata {
    tcp_listener: Arc<Mutex<TcpStream>>,
    ip_addr: IpAddr,
}

struct Server {
    server_address: String,
    conn_map: HashMap<PEER_ID, ConnectionMetadata>,
}

impl Server {
    pub fn get_server_address(&self) -> String {
        self.server_address.clone()
    }

    pub fn get_connection(&self, peer_id: &PEER_ID) -> Option<ConnectionMetadata> {
        match self.conn_map.get(peer_id) {
            Some(conn_md) => Some(conn_md.clone()),
            None => None,
        }
    }
}

impl RelayServer for Server {
    fn register(
        &mut self,
        peer_id: PEER_ID,
        source_ip: IpAddr,
        socket: Arc<Mutex<TcpStream>>,
    ) -> Result<bool, Error> {
        let conn_md = ConnectionMetadata {
            tcp_listener: socket,
            ip_addr: source_ip,
        };

        self.conn_map.insert(peer_id, conn_md);
        Ok(true)
    }

    fn connect(&mut self, target_ip: IpAddr, source_ip: IpAddr) -> Result<u128, Error> {
        todo!()
    }
}

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

    pub fn with_lock<F, T>(&self, func: F) -> T
    where
        F: FnOnce(&mut Server) -> T,
    {
        let mut lock = self.inner.try_lock().unwrap();
        let result = func(&mut *lock);
        drop(lock);
        result
    }

    pub fn register<'a>(
        &self,
        peer_id: PEER_ID,
        source_ip: IpAddr,
        socket: Arc<Mutex<TcpStream>>,
    ) -> Result<bool, Error> {
        self.with_lock(|server| server.register(peer_id, source_ip, socket))
    }

    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let server_address = self.with_lock(|server| server.get_server_address());

        let listener = TcpListener::bind(server_address.clone()).await?;

        info!("Server started on ip: {}", server_address);

        loop {
            let (mut socket, sock_addr) = listener.accept().await?;
            let cloned_self = self.clone();
            tokio::spawn(async move {
                let mut buf = vec![0; 1024];
                process_connection(socket, buf, cloned_self, sock_addr).await;
            });
        }
    }
}

async fn process_connection(
    mut socket: TcpStream,
    mut buf: Vec<u8>,
    cloned_self: MutableServerHandle,
    sock_addr: SocketAddr,
) {
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
        register_peer(socket, buf, cloned_self, sock_addr).await;
    } else if buf[0] == Operation::Relay as u8 {
        // Peer Id is 128 bytes
        read_until = read_until + 128;
        read_from_socket(&mut n, read_until, &mut socket, &mut buf).await;
        match relay_conn_time(buf, cloned_self).await {
            Ok(relay_time) => {
                let mut response: Vec<u8> = Vec::new();
                response.extend_from_slice(&(ResponseCodes::ConnectSuccess as u32).to_be_bytes());
                response.extend_from_slice(&relay_time.as_millis().to_be_bytes());

                socket.write_all(&response).await;
            }
            Err(_) => {
                socket
                    .write_all(&(ResponseCodes::ConnectFailure as u32).to_be_bytes())
                    .await;
            }
        }
    } else if buf[0] == Operation::InitiateConnect as u8 {
        // Peer Id is 128 bytes
        read_until = read_until + 128;
        read_from_socket(&mut n, read_until, &mut socket, &mut buf).await;

        // This will shutdown both the connections from two peers
        match relay_initiate_request(&buf, cloned_self).await {
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

async fn relay_initiate_request(
    buf: &Vec<u8>,
    cloned_self: MutableServerHandle,
) -> Result<(), Error> {
    let peer_id: Vec<_> = buf[1..129].iter().cloned().collect();
    let conn_md = cloned_self.with_lock(|server| server.get_connection(&peer_id));

    match conn_md {
        Some(ref co) => {
            let mut init_connect_request = Vec::new();
            init_connect_request
                .extend_from_slice(&(RequestCodes::InitConnect as u32).to_be_bytes());
            let ip_bytes = match co.ip_addr {
                IpAddr::V4(ip) => ip.octets().to_vec(),
                IpAddr::V6(ip) => ip.octets().to_vec(),
            };
            init_connect_request.extend_from_slice(&ip_bytes);

            match co
                .tcp_listener
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
                        n = co
                            .tcp_listener
                            .try_lock()
                            .unwrap()
                            .read(&mut buf_new)
                            .await
                            .expect("failed to read data from socket");
                    }

                    info!(
                        "{}",
                        format!(
                            "Peer {} has made connection sucesfully to calling peer with ip {}",
                            String::from_utf8(peer_id.clone()).unwrap(),
                            co.ip_addr.to_string()
                        )
                    );

                    // TODO: Shutdown connection
                    // co.tcp_listener.try_lock().unwrap().shutdown().await;
                }
                Err(e) => {
                    error!(
                        "{}",
                        format!(
                        "Cannot relay initiate connection request to client with socket {}. Error {}.",
                        co.ip_addr, e
                        )
                    );
                    return Err(Error::new(
                        std::io::ErrorKind::AddrNotAvailable,
                        "Cant Send connection init request to peer",
                    ));
                }
            };
        }
        None => {
            return Err(Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "Peer connection not found in hashmap",
            ));
        }
    };

    Ok(())
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

async fn relay_conn_time(
    buf: Vec<u8>,
    cloned_self: MutableServerHandle,
) -> Result<Duration, Error> {
    let peer_id: Vec<_> = buf[1..129].iter().cloned().collect();
    let ping = (RequestCodes::Ping as u32).to_be_bytes();
    let conn_md = cloned_self.with_lock(|server| server.get_connection(&peer_id));

    let mut max_tries = 3;
    let mut time_to_reach_peer = Duration::default();

    while max_tries > 0 {
        match conn_md {
            Some(ref co) => {
                let read_until_1 = 1;
                let mut n_1 = 0;
                let mut buf_1 = vec![0; 1];

                let start = Instant::now();
                match co.tcp_listener.try_lock().unwrap().write_all(&ping).await {
                    Ok(_) => {
                        // REad one byte client response

                        while n_1 <= read_until_1 {
                            n_1 = co
                                .tcp_listener
                                .try_lock()
                                .unwrap()
                                .read(&mut buf_1)
                                .await
                                .expect("failed to read data from socket");
                        }
                        // Once we got the response from peer, send the time
                        // it takes for a tcp ping request to asking client
                        time_to_reach_peer = start.elapsed();
                        break;
                    }
                    Err(e) => {
                        error!(
                            "{}",
                            format!(
                                "Cannot relay connection to socket {}. Error {}. \n Retry Left {}",
                                co.ip_addr, e, max_tries
                            )
                        );

                        thread::sleep(Duration::from_secs(1));
                        max_tries -= 1;
                    }
                };
            }
            None => {
                error!(
                    "{}",
                    format!(
                        "Cannot relay connection to socket. Conn not found in hashmap. \n Retry Left {}",
                        max_tries
                    )
                );

                thread::sleep(Duration::from_secs(1));
                max_tries -= 1;
            }
        };
    }

    if max_tries <= 0 {
        // Client connection is not reachable
        error!(
            "{}",
            format!(
                "Cannot Relay Connection to peer with peer_id: {} after retries.",
                String::from_utf8(peer_id).unwrap()
            )
        );
        return Err(Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            "Error connecting to peer",
        ));
    }

    Ok(time_to_reach_peer)
}

async fn register_peer(
    socket: TcpStream,
    buf: Vec<u8>,
    cloned_self: MutableServerHandle,
    sock_addr: SocketAddr,
) {
    let socket_to_copy = Arc::new(Mutex::new(socket));

    let peer_id: Vec<_> = buf[1..129].iter().cloned().collect();

    // self.inner.lock().unwrap().register(sock_addr.ip(), conn_md);
    match cloned_self.register(peer_id.clone(), sock_addr.ip(), socket_to_copy.clone()) {
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
}
