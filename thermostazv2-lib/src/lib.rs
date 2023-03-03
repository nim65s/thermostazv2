#![no_std]
#![feature(error_in_core)]

use heapless::Vec;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum TError {
    #[error("postcard error {0}")]
    Postcard(postcard::Error),

    #[error("sensor error {0:?}")]
    Sensor(SensorErr),
}

/// Sensor: AHT20

#[derive(Deserialize, Serialize, MaxSize, Debug, Eq, PartialEq, Copy, Clone)]
pub struct SensorOk {
    pub h: u32,
    pub t: u32,
}

impl SensorOk {
    #[must_use]
    pub fn rh(&self) -> f64 {
        100.0 * f64::from(self.h) / (f64::from(1 << 20))
    }
    #[must_use]
    pub fn celsius(&self) -> f64 {
        (200.0 * f64::from(self.t) / (f64::from(1 << 20))) - 50.0
    }
}

#[repr(u8)]
#[derive(Deserialize, Serialize, MaxSize, Debug, Eq, PartialEq, Copy, Clone)]
pub enum SensorErr {
    Uncalibrated,
    Bus,
    CheckSum,
    Uninitialized,
}

#[derive(Deserialize, Serialize, MaxSize, Debug, Eq, PartialEq, Copy, Clone)]
pub enum SensorResult {
    Err(SensorErr),
    Ok(SensorOk),
}

#[repr(u8)]
#[derive(Deserialize, Serialize, MaxSize, Debug, Eq, PartialEq, Copy, Clone)]
pub enum Relay {
    Hot,
    Cold,
}

impl From<bool> for Relay {
    fn from(val: bool) -> Self {
        if val {
            Self::Hot
        } else {
            Self::Cold
        }
    }
}

#[repr(u8)]
#[derive(Deserialize, Serialize, MaxSize, Debug, Eq, PartialEq, Copy, Clone)]
pub enum Cmd {
    Get,
    Ping,
    Pong,
    Set(Relay),
    Status(Relay, SensorResult),
}

pub type TVec = Vec<u8, { Cmd::POSTCARD_MAX_SIZE + 2 }>;

impl Cmd {
    pub fn to_vec(&self) -> Result<TVec, TError> {
        postcard::to_vec_cobs(&self).map_err(TError::Postcard)
    }

    pub fn from_vec(value: &mut [u8]) -> Result<Self, TError> {
        postcard::from_bytes_cobs(value).map_err(TError::Postcard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;

    #[test]
    fn cmd_to_vec_to_cmd() {
        let cmd_in = Cmd::Status(
            Relay::Hot,
            SensorResult::Ok(SensorOk {
                h: u32::MAX,
                t: u32::MAX,
            }),
        );

        std::dbg!(cmd_in);
        let data = cmd_in.to_vec();
        assert!(data.is_ok(), "data is not ok: {data:?}");
        let mut data = data.unwrap();
        std::dbg!(&data);
        std::println!("data len: {}", data.len());
        std::println!("max len: {}", Cmd::POSTCARD_MAX_SIZE);
        let cmd_out = Cmd::from_vec(data.as_mut());
        assert!(cmd_out.is_ok(), "cmd_out is not ok: {cmd_out:?}");
        let cmd_out = cmd_out.unwrap();
        std::dbg!(cmd_out);
        assert_eq!(cmd_out, cmd_in);
    }
}
