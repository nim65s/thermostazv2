use crate::err::ThermostazvError;
use bytes::BufMut;
use bytes::BytesMut;
use thermostazv2_lib::Cmd;
use tokio_util::codec::{Decoder, Encoder};

#[derive(Debug)]
pub struct SerialConnection {}

impl Decoder for SerialConnection {
    type Item = Cmd;
    type Error = ThermostazvError;

    #[tracing::instrument]
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        Ok(if Some(&0) == src.last() {
            tracing::trace!("decoding...");
            Some(Cmd::from_vec(src)?)
        } else {
            tracing::trace!("not enough bytes yet...");
            None
        })
    }
}

impl Encoder<Cmd> for SerialConnection {
    type Error = ThermostazvError;

    #[tracing::instrument]
    fn encode(&mut self, cmd: Cmd, buf: &mut BytesMut) -> Result<(), Self::Error> {
        tracing::trace!("encode {:?}", cmd);
        let data = cmd.to_vec()?;
        buf.reserve(data.len());
        buf.put(data.as_slice());
        Ok(())
    }
}

impl SerialConnection {
    pub const fn new() -> Self {
        Self {}
    }
}
