use std::{io::Cursor, sync::Arc};

use bytes::{BufMut, BytesMut};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
    net::TcpStream,
    sync::Mutex,
};
use tracing::info;

use crate::relay::{
    frame::{
        self, Error, Frame, Operations, PeerFrames, PeerOperations, RelayFrames, RelayOperations,
    },
    framehelper,
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
        framehelper::read_frame_given_stream(self.stream.clone(), &mut self.buffer).await
    }

    async fn write_frame(&mut self, frame: PeerFrames) -> Result<(), std::io::Error> {
        framehelper::write_frame_given_stream(self.stream.clone(), frame).await
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
    /// Buffer grows in size as the connection is being used.
    fn new(socket: Arc<Mutex<BufWriter<S>>>) -> Self {
        PeerConnection {
            stream: socket,
            buffer: BytesMut::with_capacity(1024),
        }
    }

    async fn read_frame(&mut self) -> Result<Option<PeerFrames>, std::io::Error> {
        framehelper::read_frame_given_stream(self.stream.clone(), &mut self.buffer).await
    }

    async fn write_frame(&mut self, frame: RelayFrames) -> Result<(), std::io::Error> {
        framehelper::write_frame_given_stream(self.stream.clone(), frame).await
    }
}

#[cfg(test)]
mod tests {}
