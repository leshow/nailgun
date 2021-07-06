use std::net::SocketAddr;

use bytes::Bytes;
use bytes::{Buf, BytesMut};
use tokio_util::codec::Decoder;

pub struct BufMsg {
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
    pub fn msg_id(&self) -> u16 {
        u16::from_be_bytes([self.buf[0], self.buf[1]])
    }

    /// Get the response code from the byte buffer (not using edns)
    pub fn r_code(&self) -> u8 {
        // let qr_to_rcode = u8::from_be_bytes([]);
        0x0F & self.buf[3]
    }

    // /// create a new `SerialMsg` from any `AsyncReadExt`
    // pub async fn read<S>(stream: &mut S, src: SocketAddr) -> io::Result<Self>
    // where
    //     S: Unpin + AsyncReadExt,
    // {
    //     let mut buf = [0u8; 2];
    //     let _bytes_len = stream.read_exact(&mut buf).await?;
    //     let len = u16::from_be_bytes(buf) as usize;

    //     let mut buf = vec![0; len];
    //     stream.read_exact(&mut buf).await?;

    //     Ok(Self::new(buf.into(), src))
    // }

    // /// write a `SerialMsg` to any `AsyncWriteExt`
    // pub async fn write<S>(&self, stream: &mut S) -> io::Result<usize>
    // where
    //     S: Unpin + AsyncWriteExt,
    // {
    //     let len = self.buf.len();
    //     let byte_len = (len as u16).to_be_bytes();
    //     // send
    //     stream.write_all(&byte_len).await?;
    //     stream.write_all(&self.buf).await?;

    //     Ok(len)
    // }
}

impl From<(Bytes, SocketAddr)> for BufMsg {
    fn from((buf, addr): (Bytes, SocketAddr)) -> Self {
        Self { buf, addr }
    }
}

pub struct TcpBufMsgDecoder {
    pub addr: SocketAddr,
}

impl Decoder for TcpBufMsgDecoder {
    type Item = (BytesMut, SocketAddr);
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 2 {
            // Not enough data to read length marker.
            return Ok(None);
        }

        let mut len_buf = [0u8; 2];
        len_buf.copy_from_slice(&src[..2]);
        let length = u16::from_be_bytes(len_buf) as usize;

        if src.len() < 2 + length {
            // buffer doesn't have all the data yet, so reserve
            // and read more
            src.reserve(2 + length - src.len());
            return Ok(None);
        }

        // TODO use split?
        // let buf = src[2..][..length].to_vec();
        // src.advance(2 + length);
        // Ok(Some(BufMsg::new(buf.into(), self.addr)))
        src.advance(2); // advance over len
        let buf = src.split_to(length);
        Ok(Some((buf, self.addr)))
    }
}
