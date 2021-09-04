use std::net::SocketAddr;

use bytes::Bytes;
use bytes::{Buf, BytesMut};
use tokio_util::codec::Decoder;

#[derive(Clone, Debug)]
pub struct BufMsg {
    buf: Bytes,
    addr: SocketAddr,
}

impl BufMsg {
    /// Construct a new `BufMsg` with an address
    pub fn new(buf: Bytes, addr: SocketAddr) -> Self {
        Self { buf, addr }
    }

    /// Return a reference to internal bytes
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Get associated address
    #[allow(dead_code)]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Get DNS id from the buffer
    pub fn msg_id(&self) -> u16 {
        u16::from_be_bytes([self.buf[0], self.buf[1]])
    }

    /// Get response code from byte buffer (not using edns)
    pub fn rcode(&self) -> u8 {
        0x0F & u8::from_be_bytes([self.buf[3]])
    }
}

impl From<(Bytes, SocketAddr)> for BufMsg {
    fn from((buf, addr): (Bytes, SocketAddr)) -> Self {
        Self { buf, addr }
    }
}

pub struct TcpDecoder {
    pub target: SocketAddr,
}

impl Decoder for TcpDecoder {
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

        src.advance(2); // advance over len
        let buf = src.split_to(length);
        Ok(Some((buf, self.target)))
    }
}
