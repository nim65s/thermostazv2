use crate::err::ThermostazvError;
use bincode::{decode_from_slice, encode_into_slice};
use bytes::BufMut;
use bytes::BytesMut;
use core::cmp::Ordering;
use thermostazv2_lib::{Cmd, HEADER};
use tokio_util::codec::{Decoder, Encoder};

#[derive(Debug)]
pub struct SerialConnection {
    header_index: usize,
    buffer: [u8; 32],
    buffer_index: usize,
    buffer_size: usize,
}

impl Decoder for SerialConnection {
    type Item = Cmd;
    type Error = ThermostazvError;

    #[tracing::instrument]
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        tracing::trace!("decoding...");
        for byte in src.split().iter() {
            match self.header_index.cmp(&HEADER.len()) {
                Ordering::Less => {
                    if *byte == HEADER[self.header_index] {
                        tracing::trace!("good header {}", byte);
                        self.header_index += 1;
                    } else {
                        tracing::error!("wrong header {}: {}", self.header_index, byte);
                        self.header_index = 0;
                    }
                }
                Ordering::Equal => {
                    self.buffer_index = 0;
                    self.header_index += 1;
                    self.buffer_size = (*byte).into();
                    tracing::trace!("header OK, read next {} bytes", byte);
                }
                Ordering::Greater => {
                    self.buffer[self.buffer_index] = *byte;
                    self.buffer_index += 1;
                    if self.buffer_index == self.buffer_size {
                        self.header_index = 0;
                        tracing::trace!(
                            "{} bytes were read, we should have a Cmd",
                            self.buffer_size
                        );
                        let config = bincode::config::standard();
                        return decode_from_slice(&self.buffer[..self.buffer_size], config)
                            .map(|(ret, _)| Some(ret))
                            .map_err(|e| {
                                ThermostazvError::Bincode(format!("decode error: {e:?}"))
                            });
                    }
                }
            }
        }
        Ok(None)
    }
}

impl Encoder<Cmd> for SerialConnection {
    type Error = ThermostazvError;

    #[tracing::instrument]
    fn encode(&mut self, cmd: Cmd, buf: &mut BytesMut) -> Result<(), Self::Error> {
        tracing::trace!("encode {:?}", cmd);
        let mut dst = [0; 32];
        let config = bincode::config::standard();
        let size = encode_into_slice(cmd, &mut dst, config)
            .map_err(|e| ThermostazvError::Bincode(format!("encode error: {e:?}")))?;
        buf.reserve(size + 5);
        buf.put(&HEADER[..]);
        buf.put_u8(
            size.try_into()
                .map_err(|e| ThermostazvError::Bincode(format!("encode error: {e:?}")))?,
        );
        buf.put(&dst[..size]);
        Ok(())
    }
}

impl SerialConnection {
    pub const fn new() -> Self {
        Self {
            header_index: 0,
            buffer: [0; 32],
            buffer_index: 0,
            buffer_size: 0,
        }
    }
}
