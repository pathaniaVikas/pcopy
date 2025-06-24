use std::{io::Cursor, sync::Arc};

use bytes::{BufMut, BytesMut};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
    net::TcpStream,
    sync::Mutex,
};
use tracing::info;

use crate::relay::frame::{
    self, Error, Frame, Operations, PeerFrames, PeerOperations, RelayFrames, RelayOperations,
};

/// Stream, Buffer, ReceivingFrame, SendingFrame, ReceivingOperations, SendingOperations, Error
pub trait Connection<S, B, RF, SF, RO, SO, E>
where
    S: Sync + Send + Clone,
    B: BufMut,
    E: std::error::Error,
    RO: Operations,
    SO: Operations,
    RF: Frame<RO, frame::Error>,
    SF: Frame<SO, frame::Error>,
{
    fn new(socket: S) -> Self;
    async fn read_frame(&mut self) -> Result<Option<RF>, E>;
    async fn write_frame(&mut self, frame: SF) -> Result<(), E>;
}

#[derive(Debug)]
pub struct RelayConnection<S>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin + Send + Sync + 'static,
{
    pub stream: Arc<Mutex<BufWriter<S>>>,
    buffer: BytesMut,
}

impl<S>
    Connection<
        Arc<Mutex<BufWriter<S>>>,
        BytesMut,
        RelayFrames,
        PeerFrames,
        RelayOperations,
        PeerOperations,
        std::io::Error,
    > for RelayConnection<S>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin + Send + Sync + 'static,
{
    fn new(socket: Arc<Mutex<BufWriter<S>>>) -> Self {
        RelayConnection {
            stream: socket,
            buffer: BytesMut::with_capacity(1024),
        }
    }

    async fn read_frame(&mut self) -> Result<Option<RelayFrames>, std::io::Error> {
        Self::read_frame_from_stream(self.stream.clone(), &mut self.buffer).await
    }

    async fn write_frame(&mut self, frame: PeerFrames) -> Result<(), std::io::Error> {
        Self::write_frame_to_stream(self.stream.clone(), frame).await
    }
}

impl<S> RelayConnection<S>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin + Send + Sync + 'static,
{
    pub async fn read_frame_from_stream(
        stream: Arc<Mutex<BufWriter<S>>>,
        mut buffer: &mut BytesMut,
    ) -> Result<Option<RelayFrames>, std::io::Error> {
        loop {
            if let Some(frame) = Self::parse_frame(&buffer)? {
                info!("*** Found frame in stream");
                return Ok(Some(frame));
            }
            // There is not enough buffered data to read a frame. Attempt to
            // read more data from the socket.
            //
            // On success, the number of bytes is returned. `0` indicates "end
            // of stream".
            // TODO: Should we use channels to prevent race conditions
            info!("*** Reading from stream");
            if 0 == stream.lock().await.read(&mut buffer).await? {
                // The remote closed the connection. For this to be a clean
                // shutdown, there should be no data in the read buffer. If
                // there is, this means that the peer closed the socket while
                // sending a frame.
                if buffer.is_empty() {
                    info!("*** Stream got closed");
                    return Ok(None);
                } else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        "connection reset by peer",
                    ));
                }
            } else {
                info!("REad more than 0 bytes")
            }
        }
    }

    fn parse_frame(buffer: &BytesMut) -> Result<Option<RelayFrames>, std::io::Error> {
        let mut buf = Cursor::new(&buffer[..]);
        match RelayFrames::parse(&mut buf) {
            Ok(frame) => {
                info!("*** Successfully parsed frame");
                return Ok(Some(frame));
            }
            Err(Error::Incomplete) => return Ok(None),
            Err(e) => return Err(e.into()),
        }
    }

    pub async fn write_frame_to_stream(
        stream: Arc<Mutex<BufWriter<S>>>,
        frame: PeerFrames,
    ) -> Result<(), std::io::Error> {
        let mut lock = stream.lock().await;
        lock.write_all(&frame.to_be_bytes()).await?;
        lock.flush().await?;
        drop(lock);
        Ok(())
    }
}

pub struct PeerConnection<S>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin + Send + Sync + 'static,
{
    pub stream: Arc<Mutex<BufWriter<S>>>,
    buffer: BytesMut,
}

impl<S>
    Connection<
        Arc<Mutex<BufWriter<S>>>,
        BytesMut,
        PeerFrames,
        RelayFrames,
        PeerOperations,
        RelayOperations,
        std::io::Error,
    > for PeerConnection<S>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin + Send + Sync + 'static,
{
    fn new(socket: Arc<Mutex<BufWriter<S>>>) -> Self {
        PeerConnection {
            stream: socket,
            buffer: BytesMut::with_capacity(1024),
        }
    }

    async fn read_frame(&mut self) -> Result<Option<PeerFrames>, std::io::Error> {
        Self::read_frame_from_stream(self.stream.clone(), &mut self.buffer).await
    }

    async fn write_frame(&mut self, frame: RelayFrames) -> Result<(), std::io::Error> {
        Self::write_frame_to_stream(self.stream.clone(), frame).await
    }
}

impl<S> PeerConnection<S>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin + Send + Sync + 'static,
{
    pub async fn read_frame_from_stream(
        stream: Arc<Mutex<BufWriter<S>>>,
        mut buffer: &mut BytesMut,
    ) -> Result<Option<PeerFrames>, std::io::Error> {
        loop {
            if let Some(frame) = Self::parse_frame(&buffer)? {
                info!("*** Found frame in stream");
                return Ok(Some(frame));
            }
            // There is not enough buffered data to read a frame. Attempt to
            // read more data from the socket.
            info!("*** Reading from stream");
            if 0 == stream.lock().await.read(&mut buffer).await? {
                // The remote closed the connection. For this to be a clean
                // shutdown, there should be no data in the read buffer. If
                // there is, this means that the peer closed the socket while
                // sending a frame.
                if buffer.is_empty() {
                    info!("*** Stream got closed");
                    return Ok(None);
                } else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        "connection reset by peer",
                    ));
                }
            } else {
                info!("REad more than 0 bytes")
            }
        }
    }

    fn parse_frame(buffer: &BytesMut) -> Result<Option<PeerFrames>, std::io::Error> {
        let mut buf = Cursor::new(&buffer[..]);
        match PeerFrames::parse(&mut buf) {
            Ok(frame) => {
                info!("*** Successfully parsed frame");
                return Ok(Some(frame));
            }
            Err(Error::Incomplete) => return Ok(None),
            Err(e) => return Err(e.into()),
        }
    }

    async fn write_frame_to_stream(
        stream: Arc<Mutex<BufWriter<S>>>,
        frame: RelayFrames,
    ) -> Result<(), std::io::Error> {
        let mut lock = stream.lock().await;
        lock.write_all(&frame.to_be_bytes()).await?;
        lock.flush().await?;
        drop(lock);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::frame::{
        PeerCommands, PeerFrames, RelayFrames, RelayOperations, StatusCodes,
    };
    use crate::relay::peer::PeerId;
    use bytes::BytesMut;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{BufWriter, DuplexStream};
    use tokio::net::TcpStream;
    use tokio::sync::Mutex;
    use tokio::time;
    use tracing::info;

    async fn dummy_stream() -> (Arc<Mutex<BufWriter<DuplexStream>>>, DuplexStream) {
        let (client, server) = tokio::io::duplex(1024);
        let client = Arc::new(Mutex::new(BufWriter::new(client)));
        (client, server)
    }

    #[tokio::test]
    async fn test_write_frame_to_stream() {
        let (stream, mut server) = dummy_stream().await;
        let frame = PeerFrames::RegisterResult(StatusCodes::SUCCESS);

        RelayConnection::<DuplexStream>::write_frame_to_stream(stream.clone(), frame)
            .await
            .expect("write_frame_to_stream should succeed");

        // Read from the server end to verify bytes were written
        let mut buf = [0u8; 8];
        let n = server.read(&mut buf).await.expect("read from server");
        assert!(n > 0);
        // The first byte should be the operation code for RegisterResult
        assert_eq!(
            buf[0],
            crate::relay::frame::PeerOperations::RegisterResult as u8
        );
        // The second byte should be the StatusCodes::SUCCESS as u8
        assert_eq!(buf[1], StatusCodes::SUCCESS.into_u8());
    }

    #[tokio::test]
    async fn test_parse_frame_success() {
        tracing_subscriber::fmt::try_init().ok();
        // Prepare a buffer with a valid ReceivingOperation (e.g., Ping)
        let mut buffer = BytesMut::new();
        buffer.extend_from_slice(&[RelayOperations::Ping as u8]);
        // You may need to extend with more bytes if RecevingFrames::parse expects more

        let result = RelayConnection::<DuplexStream>::parse_frame(&buffer);
        assert!(result.is_ok());
        let frame = result.unwrap();
        // If parse returns Some, check the variant
        if let Some(RelayFrames::Ping) = frame {
            // Success
        } else {
            panic!("Expected RecevingFrames::Ping");
        }
    }

    #[tokio::test]
    async fn test_parse_frame_incomplete() {
        // Prepare an empty buffer
        let buffer = BytesMut::new();
        let result = RelayConnection::<DuplexStream>::parse_frame(&buffer);
        // Should be Ok(None) for incomplete
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_parse_frame_invalid_operation() {
        // Prepare a buffer with an invalid operation byte
        let mut buffer = BytesMut::new();
        buffer.extend_from_slice(&[0xFF]); // 0xFF is not a valid ReceivingOperations
        let result = RelayConnection::<DuplexStream>::parse_frame(&buffer);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_parse_frame_valid_register() {
        // Prepare a buffer for a Register frame
        // You may need to adjust the payload depending on your RecevingFrames::parse implementation
        let peer_id = PeerId::default(); // Example PeerId bytes
        let mut buffer = BytesMut::new();
        buffer.extend_from_slice(&[RelayOperations::Register as u8]);
        buffer.extend_from_slice(&peer_id.as_bytes());
        assert!(buffer.len() >= 65); // Ensure buffer has enough bytes for operation + peer_id
        let result = RelayConnection::<DuplexStream>::parse_frame(&buffer);
        assert!(result.is_ok());
        if let Some(RelayFrames::Register(_peer_id)) = result.unwrap() {
            // Optionally check peer_id bytes here
        } else {
            panic!("Expected RecevingFrames::Register");
        }
    }

    #[tokio::test]
    async fn test_parse_frame_valid_probe() {
        // Prepare a buffer for a Probe frame
        let peer_id = PeerId::default(); // Example PeerId bytes
        let mut buffer = BytesMut::new();
        buffer.extend_from_slice(&[RelayOperations::Probe as u8]);
        buffer.extend_from_slice(&peer_id.as_bytes());
        assert!(buffer.len() >= 65); // Ensure buffer has enough bytes for operation + peer_id
        let result = RelayConnection::<DuplexStream>::parse_frame(&buffer);
        assert!(result.is_ok());
        if let Some(RelayFrames::Probe(_peer_id)) = result.unwrap() {
            // Optionally check peer_id bytes here
        } else {
            panic!("Expected RecevingFrames::Probe");
        }
    }

    // #[tokio::test]
    // async fn test_read_frame_from_stream_ping() {
    //     tracing_subscriber::fmt::try_init().ok();
    //     // let (client, mut server) = dummy_stream().await;
    //     // let (mut client, mut server) = tokio::io::duplex(1024);
    //     let mut stream = TcpStream::connect("127.0.0.1:8080").await.unwrap();
    //     let shared_stream = Arc::new(Mutex::new(BufWriter::new(stream)));

    //     // Write a Ping frame to the server side of the duplex stream
    //     let ping_bytes = [ReceivingOperations::Ping as u8];
    //     shared_stream
    //         .lock()
    //         .await
    //         .write_all(&ping_bytes)
    //         .await
    //         .expect("write to server");

    //     // client.flush().await.expect("flush server");
    //     // Now try to read the frame from the client side using read_frame_from_stream
    //     let mut buffer = BytesMut::with_capacity(1);
    //     let n = shared_stream
    //         .lock()
    //         .await
    //         .read(&mut buffer)
    //         .await
    //         .expect("read from client");
    //     assert!(n > 0);
    //     assert!(!buffer.is_empty());
    //     assert_eq!(buffer[0], ReceivingOperations::Ping as u8);

    //     // let result =
    //     //     PeerConnection::<DuplexStream>::read_frame_from_stream(client.clone(), &mut buffer)
    //     //         .await
    //     //         .expect("read_frame_from_stream should succeed");
    //     // assert!(result.is_some());
    //     // assert!(matches!(result, Some(RecevingFrames::Ping)));
    // }

    // #[tokio::test]
    // async fn test_read_frame_from_stream_register() {
    //     tracing_subscriber::fmt::try_init().ok();
    //     let (client, mut server) = dummy_stream().await;

    //     // Prepare a Register frame: [operation_byte, peer_id_bytes...]
    //     let peer_id_bytes = [0x01, 0x02, 0x03, 0x04]; // Example PeerId bytes
    //     let mut register_bytes = vec![ReceivingOperations::Register as u8];
    //     register_bytes.extend_from_slice(&peer_id_bytes);
    //     // Write the Register frame to the server side of the duplex stream
    //     server
    //         .write_all(&register_bytes)
    //         .await
    //         .expect("write to server");

    //     server.flush().await.expect("flush server");

    //     // Now try to read the frame from the client side using read_frame_from_stream
    //     let mut buffer = BytesMut::with_capacity(1024);

    //     let join_handle = tokio::spawn(async move {
    //         info!("*** Tokio spawn started");
    //         let result =
    //             PeerConnection::<DuplexStream>::read_frame_from_stream(client.clone(), &mut buffer)
    //                 .await
    //                 .expect("read_frame_from_stream should succeed");

    //         if let Some(RecevingFrames::Register(peer_id)) = result {
    //             assert_eq!(peer_id, PeerId::new(&peer_id_bytes, 0));
    //         } else {
    //             panic!("Expected RecevingFrames::Register");
    //         }
    //     });
    //     // register_bytes.extend_from_slice(&peer_id_bytes);
    //     join_handle.await.unwrap();
    //     time::sleep(Duration::from_millis(1000)).await;
    // }
}
