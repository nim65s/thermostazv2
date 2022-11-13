#![no_std]

use bincode::{Decode, Encode};

/// Sensor: AHT20

#[derive(Encode, Decode, Debug, Eq, PartialEq)]
pub struct SensorOk {
    pub h: u32,
    pub t: u32,
}

impl SensorOk {
    pub fn rh(&self) -> f32 {
        100.0 * (self.h as f32) / ((1 << 20) as f32)
    }
    pub fn celsius(&self) -> f32 {
        (200.0 * (self.t as f32) / ((1 << 20) as f32)) - 50.0
    }
}

#[derive(Encode, Decode, Debug, Eq, PartialEq)]
pub enum SensorErr {
    Uncalibrated,
    Bus,
    CheckSum,
}

#[derive(Encode, Decode, Debug, Eq, PartialEq)]
pub enum SensorResult {
    Err(SensorErr),
    Ok(SensorOk),
}

#[derive(Encode, Decode, Debug, Eq, PartialEq)]
pub enum Relay {
    Open,
    Closed,
    Batman,
}

#[derive(Encode, Decode, Debug, Eq, PartialEq)]
pub enum Cmd {
    Get,
    Ping,
    Set(Relay),
    Sensor(SensorResult),
    Status(Relay, SensorResult),
    //Syn(Handshake),
}

pub static HEADER: [u8; 4] = [0xFF, 0xFF, 0xFD, 0x00];
