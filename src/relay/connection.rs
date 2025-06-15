use std::{io::Cursor, sync::Arc};

use bytes::{BufMut, BytesMut};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
    net::TcpStream,
    sync::Mutex,
};

use crate::relay::frame::{self, Error, Frame, RecevingFrames, SendingFrames};

// Steram, Buffer, ReceivingFrame, SendingFrame, Error
pub trait Connection<S, B, RF, SF, E>
where
    S: Sync + Send + Clone,
    B: BufMut,
{
    fn new(socket: S) -> Self;
    async fn read_frame(&mut self) -> Result<Option<RF>, E>;
    async fn write_frame(&mut self, sf: SF) -> Result<(), E>;
}

#[derive(Debug)]
pub struct PeerConnection {
    pub stream: Arc<Mutex<BufWriter<TcpStream>>>,
    buffer: BytesMut,
}

impl
    Connection<
        Arc<Mutex<BufWriter<TcpStream>>>,
        BytesMut,
        RecevingFrames,
        SendingFrames,
        std::io::Error,
    > for PeerConnection
{
    fn new(socket: Arc<Mutex<BufWriter<TcpStream>>>) -> Self {
        PeerConnection {
            stream: socket,
            buffer: BytesMut::with_capacity(1024),
        }
    }

    async fn read_frame(&mut self) -> Result<Option<RecevingFrames>, std::io::Error> {
        Self::read_frame_from_stream(self.stream.clone(), &mut self.buffer).await
    }

    async fn write_frame(&mut self, sf: SendingFrames) -> Result<(), std::io::Error> {
        Self::write_frame_to_stream(self.stream.clone(), sf).await
    }
}

impl PeerConnection {
    pub async fn read_frame_from_stream(
        stream: Arc<Mutex<BufWriter<TcpStream>>>,
        mut buffer: &mut BytesMut,
    ) -> Result<Option<RecevingFrames>, std::io::Error> {
        loop {
            if let Some(frame) = Self::parse_frame(&buffer)? {
                return Ok(Some(frame));
            }

            // There is not enough buffered data to read a frame. Attempt to
            // read more data from the socket.
            //
            // On success, the number of bytes is returned. `0` indicates "end
            // of stream".
            // TODO: Should we use channels to prevent race conditions
            if 0 == stream.lock().await.read(&mut buffer).await? {
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

    fn parse_frame(buffer: &BytesMut) -> Result<Option<RecevingFrames>, std::io::Error> {
        let mut buf = Cursor::new(&buffer[..]);
        match RecevingFrames::read_operation(&mut buf) {
            Ok(operation) => {
                let frame = RecevingFrames::parse(&mut buf, operation)
                    .map_err(|e| <frame::Error as Into<std::io::Error>>::into(e))?;
                Ok(Some(frame))
            }
            Err(Error::Incomplete) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn write_frame_to_stream(
        stream: Arc<Mutex<BufWriter<TcpStream>>>,
        frame: SendingFrames,
    ) -> Result<(), std::io::Error> {
        let mut lock = stream.lock().await;
        lock.write_all(&frame.to_be_bytes()).await?;
        lock.flush().await?;
        drop(lock);
        Ok(())
    }
}
