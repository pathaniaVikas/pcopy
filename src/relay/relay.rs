use std::{collections::HashMap, io::{Error, Read}, net::{IpAddr, Ipv4Addr, SocketAddr}};

use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::{TcpListener, TcpStream}};
use tracing::{error, info};


struct ConnectionMetadata<'a>{
    tcp_listener: &'a TcpStream,
    socket_addr: SocketAddr
}

trait RelayServer {
    fn register(&mut self, source_ip: &IpAddr, conn_metadata: &ConnectionMetadata) -> Result<bool,Error>;
    fn connect(&mut self, target_ip: &IpAddr, source_ip: IpAddr) -> Result<u128, Error>;
}

enum Operation {
    // 01
    Register = 0x01,
    // 02
    Connect = 0x02,
}

struct Server<'a> {
    server_address: String,
    conn_map: HashMap<IpAddr, ConnectionMetadata<'a>>
}

impl Server<'_> {

    pub fn new(addr: String) -> Result<Server<'static>, Error> {
        Ok(Server {
            server_address: addr,
            conn_map: HashMap::new()
        })
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {

        let listener = TcpListener::bind(&self.server_address).await?;

        info!("Server started on ip: {}", self.server_address);

        loop {
            let (mut socket, sock_addr) = listener.accept().await?;

            tokio::spawn(async move {
                let conn_md = ConnectionMetadata {
                    tcp_listener: &socket,
                    socket_addr: sock_addr
                };

                let mut buf = vec![0; 1024];
                
                let mut n = 0;

                while n<=5 {
                    // Keep the connection open
                    n = socket
                    .read(&mut buf)
                    .await
                    .expect("failed to read data from socket");
                }
                
                if buf[0] == Operation::Register as u8 {
                    self.register(&sock_addr.ip(), &conn_md);
                }


            });
        }
    }
}

impl RelayServer for Server<'_> {
    fn register(&mut self, source_ip: &IpAddr, conn_metadata: &ConnectionMetadata) -> Result<bool,Error> {
        self.conn_map.insert(*source_ip, *conn_metadata);
        Ok(true)
    }

    fn connect(&mut self, target_ip: &IpAddr, source_ip: IpAddr) -> Result<u128, Error> {
        todo!()
    }
}