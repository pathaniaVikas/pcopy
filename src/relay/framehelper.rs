use std::{io::Cursor, sync::Arc};

use bytes::BytesMut;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
    sync::Mutex,
};

use crate::relay::frame::{self, Error, Frame, Operations};

pub async fn read_frame_given_stream<S, F, O>(
    stream: Arc<Mutex<BufWriter<S>>>,
    mut buffer: &mut BytesMut,
) -> Result<Option<F>, std::io::Error>
where
    S: AsyncWriteExt + AsyncReadExt + Unpin,
    O: Operations,
    F: Frame<O, frame::Error>,
{
    loop {
        if let Some(frame) = parse_frame(&buffer)? {
            return Ok(Some(frame));
        }
        // There is not enough buffered data to read a frame. Attempt to
        // read more data from the socket.
        //
        // On success, the number of bytes is returned. `0` indicates "end
        // of stream".
        // TODO: Should we use channels to prevent race conditions
        if 0 == stream.lock().await.read_buf(&mut buffer).await? {
            // The remote closed the connection. For this to be a clean
            // shutdown, there should be no data in the read buffer. If
            // there is, this means that the peer closed the socket while
            // sending a frame.
            if buffer.is_empty() {
                return Ok(None);
            } else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "connection reset by peer",
                ));
            }
        }
    }
}

pub async fn write_frame_given_stream<S, F, O>(
    stream: Arc<Mutex<BufWriter<S>>>,
    frame: F,
) -> Result<(), std::io::Error>
where
    S: AsyncWriteExt + AsyncReadExt + Unpin,
    O: Operations,
    F: Frame<O, frame::Error>,
{
    let mut lock = stream.lock().await;
    lock.write_all(&frame.to_be_bytes()).await?;
    lock.flush().await?;
    drop(lock);
    Ok(())
}

fn parse_frame<F, O>(buffer: &BytesMut) -> Result<Option<F>, std::io::Error>
where
    O: Operations,
    F: Frame<O, frame::Error>,
{
    let mut buf = Cursor::new(&buffer[..]);
    match F::parse(&mut buf) {
        Ok(frame) => {
            return Ok(Some(frame));
        }
        Err(Error::Incomplete) => {
            return Ok(None);
        }
        Err(e) => return Err(e.into()),
    }
}

mod Tests {

    use super::*;
    use crate::relay::frame::{
        PeerCommands, PeerFrames, RelayFrames, RelayOperations, StatusCodes,
    };
    use crate::relay::peer::PeerId;
    use base64::prelude::BASE64_STANDARD;
    use base64::Engine;
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

        write_frame_given_stream(stream.clone(), frame)
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
    async fn test_read_frame_from_stream_register() {
        tracing_subscriber::fmt::try_init().ok();
        let (client, mut server) = dummy_stream().await;

        // Prepare a Register frame: [operation_byte, peer_id_bytes...]
        let mut register_bytes = vec![RelayOperations::Register as u8];

        // Example PeerId bytes
        let peer_id_bytes =
            BASE64_STANDARD.encode("Peer12345678987654321_999999999999_000000000001212111");

        register_bytes.extend_from_slice(&peer_id_bytes.clone().into_bytes());

        // Write the Register frame to the server side of the duplex stream
        server.write_all(&register_bytes).await.unwrap();
        // Now try to read the frame from the client side using read_frame_from_stream
        let mut buffer = BytesMut::with_capacity(1024);

        let result = read_frame_given_stream(client.clone(), &mut buffer)
            .await
            .expect("read_frame_from_stream should succeed");

        if let Some(RelayFrames::Register(peer_id)) = result {
            assert_eq!(peer_id, PeerId::new(&peer_id_bytes.into_bytes(), 0));
        } else {
            panic!("Expected RecevingFrames::Register");
        }
    }

    #[tokio::test]
    async fn test_parse_frame_success() {
        tracing_subscriber::fmt::try_init().ok();
        // Prepare a buffer with a valid ReceivingOperation (e.g., Ping)
        let mut buffer = BytesMut::new();
        buffer.extend_from_slice(&[RelayOperations::Ping as u8]);
        // You may need to extend with more bytes if RecevingFrames::parse expects more

        let result = parse_frame(&buffer);
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
        let result = parse_frame::<RelayFrames, RelayOperations>(&buffer);
        // Should be Ok(None) for incomplete
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_parse_frame_invalid_operation() {
        // Prepare a buffer with an invalid operation byte
        let mut buffer = BytesMut::new();
        buffer.extend_from_slice(&[0xFF]); // 0xFF is not a valid ReceivingOperations
        let result = parse_frame::<RelayFrames, RelayOperations>(&buffer);
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
        let result = parse_frame::<RelayFrames, RelayOperations>(&buffer);
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
        let result = parse_frame::<RelayFrames, RelayOperations>(&buffer);
        assert!(result.is_ok());
        if let Some(RelayFrames::Probe(_peer_id)) = result.unwrap() {
            // Optionally check peer_id bytes here
        } else {
            panic!("Expected RecevingFrames::Probe");
        }
    }
}
