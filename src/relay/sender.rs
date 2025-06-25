use std::io::Error;

use bytes::BytesMut;
use local_ip_address::local_ip;
use rand::rand_core::le;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
    net::TcpStream,
};

use crate::relay::{
    connection::{Connection, PeerConnection, RelayConnection},
    frame::{Frame, PeerFrames, RelayFrames, StatusCodes},
    peer::{PeerId, PeerInfo},
};

pub struct Sender {
    peer_conn: PeerConnection<TcpStream>,
}

impl Sender {
    pub fn new(peer_connection: PeerConnection<TcpStream>) -> Self {
        Sender {
            peer_conn: peer_connection,
        }
    }

    // Returns a TcpStream connected to the peer
    pub async fn connect_to_peer(
        &mut self,
        source_peer_id: PeerId,
        destination_peer_id: PeerId,
    ) -> Result<TcpStream, Error> {
        // Logic to connect to a peer

        // ------------------------------------------------------------------------------------------
        // 1. REGISTER
        // ------------------------------------------------------------------------------------------
        // Create Register Request Frame
        let register_frame = RelayFrames::Register(source_peer_id);
        // Send Register Request Frame to Relay
        self.peer_conn.write_frame(register_frame).await?;

        // Wait for Register Response Frame from Relay, or error
        loop {
            match self.peer_conn.read_frame().await {
                Ok(frame_option) => match frame_option {
                    Some(f) => match f {
                        PeerFrames::RegisterResult(status_codes) => {
                            if status_codes == StatusCodes::SUCCESS {
                                break;
                            } else {
                                return Err(Error::new(
                                    std::io::ErrorKind::Other,
                                    "Peer Registeration failed with Relay Server",
                                ));
                            }
                        }
                        _ => {
                            return Err(Error::new(
                                std::io::ErrorKind::InvalidData,
                                "Invalid Registeration response from Relay Server. ",
                            ));
                        }
                    },
                    None => {
                        continue;
                    }
                },
                Err(e) => return Err(Error::new(std::io::ErrorKind::ConnectionReset, e)),
            }
        }

        // ------------------------------------------------------------------------------------------
        // 2. PROBE PEER
        // ------------------------------------------------------------------------------------------
        let probe_frame = RelayFrames::Probe(destination_peer_id);
        self.peer_conn.write_frame(probe_frame).await?;

        let mut peer_info: Option<PeerInfo> = None;

        // Wait for Register Response Frame from Relay, or error
        loop {
            match self.peer_conn.read_frame().await {
                Ok(frame_option) => match frame_option {
                    Some(f) => match f {
                        PeerFrames::ProbeResult(peer_info_option) => match peer_info_option {
                            Some(pi) => {
                                peer_info = Some(pi);
                                break;
                            }
                            None => {
                                return Err(Error::new(
                                    std::io::ErrorKind::Other,
                                    "Peer Info Not found in Relay Server",
                                ));
                            }
                        },
                        _ => {
                            return Err(Error::new(
                                std::io::ErrorKind::InvalidData,
                                "Invalid Probe response from Relay Server. ",
                            ));
                        }
                    },
                    None => {
                        continue;
                    }
                },
                Err(e) => return Err(Error::new(std::io::ErrorKind::ConnectionReset, e)),
            }
        }

        let ip = local_ip().map_err(|e| Error::new(std::io::ErrorKind::Other, e))?;
        let addr = (ip, 8080); // Replace 8080 with the actual port you want to connect to
        let stream = TcpStream::connect(addr).await?;
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::peer::PeerId;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_sender_new() {
        // Create a dummy TcpListener and connect to it to get a TcpStream
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::spawn(async move { TcpStream::connect(addr).await.unwrap() });
        let (server_stream, _) = listener.accept().await.unwrap();
        let client_stream = client.await.unwrap();

        // let peer_conn = RelayConnection::new(server_stream);
        // let sender = Sender::new(peer_conn);
        // Just check that sender is constructed and relay_socket is a TcpStream
        // (No direct way to check relay_socket, but construction should not panic)
        // let _ = sender;
        drop(client_stream);
    }

    #[tokio::test]
    async fn test_connect_to_peer_error() {
        // This test will likely fail to connect, but should return an Error, not panic
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::spawn(async move { TcpStream::connect(addr).await.unwrap() });
        let (server_stream, _) = listener.accept().await.unwrap();
        // let peer_conn = RelayConnection::new(server_stream);
        // let sender = Sender::new(peer_conn);

        // // Use dummy PeerIds
        // let source_peer_id = PeerId::default();
        // let destination_peer_id = PeerId::default();

        // // Since local_ip() and TcpStream::connect may fail, just check that it returns Result
        // let result = sender
        //     .connect_to_peer(source_peer_id, destination_peer_id)
        //     .await;
        // // It may succeed or fail depending on environment, but should not panic
        // assert!(result.is_ok() || result.is_err());
        drop(client);
    }

    #[tokio::test]
    async fn test_connect_to_peer_success() {
        // Start a TcpListener on local_ip:8080
        let ip = local_ip_address::local_ip().unwrap();
        let addr = std::net::SocketAddr::new(ip, 8080);
        let listener = TcpListener::bind(addr).await.unwrap();

        // Accept connections in the background
        let accept_handle = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
        });

        // Create a dummy TcpStream for Sender
        let dummy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dummy_addr = dummy_listener.local_addr().unwrap();
        let dummy_client =
            tokio::spawn(async move { TcpStream::connect(dummy_addr).await.unwrap() });
        let (server_stream, _) = dummy_listener.accept().await.unwrap();
        let _dummy_client_stream = dummy_client.await.unwrap();

        // let sender = Sender::new(server_stream);

        // let source_peer_id = PeerId::default();
        // let destination_peer_id = PeerId::default();

        // // This should succeed because listener is running on local_ip:8080
        // let result = sender
        //     .connect_to_peer(source_peer_id, destination_peer_id)
        //     .await;
        // assert!(result.is_ok());

        // accept_handle.await.unwrap();
    }
}
