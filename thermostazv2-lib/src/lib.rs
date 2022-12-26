#![no_std]

use bincode::{Decode, Encode};

/// Sensor: AHT20

#[derive(Encode, Decode, Debug, Eq, PartialEq, Copy, Clone)]
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

#[derive(Encode, Decode, Debug, Eq, PartialEq, Copy, Clone)]
pub enum SensorErr {
    Uncalibrated,
    Bus,
    CheckSum,
    Uninitialized,
}

#[derive(Encode, Decode, Debug, Eq, PartialEq, Copy, Clone)]
pub enum SensorResult {
    Err(SensorErr),
    Ok(SensorOk),
}

#[derive(Encode, Decode, Debug, Eq, PartialEq, Copy, Clone)]
pub enum Relay {
    Hot,
    Cold,
}

#[derive(Encode, Decode, Debug, Eq, PartialEq, Copy, Clone)]
pub enum Cmd {
    Get,
    Ping,
    Pong,
    Set(Relay),
    Status(Relay, SensorResult),
}

pub static HEADER: [u8; 4] = [0xFF, 0xFF, 0xFD, 0x00];
