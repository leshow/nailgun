use std::{io, net::SocketAddr};

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) struct BufMsg {
    buf: Bytes,
    addr: SocketAddr,
}

impl BufMsg {
    /// Construct a new `SerialMsg` and the source or destination address
    pub fn new(buf: Bytes, addr: SocketAddr) -> Self {
        BufMsg { buf, addr }
    }

    /// Get a reference to the bytes
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Get the source or destination address (context dependent)
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Get dns message id from the buffer
    pub(crate) fn msg_id(&self) -> u16 {
        u16::from_be_bytes([self.buf[0], self.buf[1]])
    }

    /// Get the response code from the byte buffer (not using edns)
    pub fn r_code(&self) -> u8 {
        let qr_to_rcode = u8::from_be_bytes([self.buf[3]]);
        0b0000_1111 & qr_to_rcode
    }

    /// create a new `SerialMsg` from any `AsyncReadExt`
    pub async fn read<S>(stream: &mut S, src: SocketAddr) -> io::Result<Self>
    where
        S: Unpin + AsyncReadExt,
    {
        let mut buf = [0u8; 2];
        let _bytes_len = stream.read_exact(&mut buf).await?;
        let len = u16::from_be_bytes(buf) as usize;

        let mut buf = vec![0; len];
        stream.read_exact(&mut buf).await?;

        Ok(Self::new(buf.into(), src))
    }

    /// write a `SerialMsg` to any `AsyncWriteExt`
    pub async fn write<S>(&self, stream: &mut S) -> io::Result<usize>
    where
        S: Unpin + AsyncWriteExt,
    {
        let len = self.buf.len();
        let byte_len = (len as u16).to_be_bytes();
        // send
        stream.write_all(&byte_len).await?;
        stream.write_all(&self.buf).await?;

        Ok(len)
    }

    // TODO: if you use this, use a shared BytesMut so we don't have to do the extra
    // alloc, or remove Bytes altogether
    // Receive a `SerialMsg` from any new `UdpRecv`
    // pub async fn recv<S>(stream: &S) -> io::Result<Self>
    // where
    //     S: Borrow<UdpSocket>,
    // {
    //     let mut buf = [0u8; 4096];
    //     let (len, src) = stream.borrow().recv_from(&mut buf).await?;
    //     Ok(Self::new(buf[..len].to_vec().into(), src))
    // }
}

impl From<(Bytes, SocketAddr)> for BufMsg {
    fn from((buf, addr): (Bytes, SocketAddr)) -> Self {
        Self { buf, addr }
    }
}
