#![no_std]

//extern crate alloc;

//use alloc::boxed::Box;
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

//#[derive(Encode, Decode, Debug, Eq, PartialEq)]
//pub enum Handshake {
//Syn(u8),
//SynAck(u8, u8),
//Ack(u8, u8),
//}

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

//pub struct Connection {
//initiator: bool,
//a: u8,
//b: u8,
//established: bool,
//generate_u8: Box<dyn Fn() -> u8>,
//}

//impl Connection {
//pub fn new(initiator: bool, generate_u8: Box<dyn Fn() -> u8>) -> Connection {
//Connection {
//initiator,
//a: 0,
//b: 0,
//established: false,
//generate_u8,
//}
//}

//pub fn connected(&self) -> bool {
//self.established
//}

//pub fn restart(&mut self) -> Cmd {
//self.established = false;
//self.initiator = true;
//self.a = (self.generate_u8)();
//Cmd::Syn(Handshake::Syn(self.a))
//}

//pub fn connect(&mut self, recv: Handshake) -> Cmd {
//self.established = false;
//match recv {
//Handshake::Syn(a) => {
//self.initiator = false;
//self.a = a;
//self.b = (self.generate_u8)();
//Cmd::Syn(Handshake::SynAck(self.a + 1, self.b))
//}
//Handshake::SynAck(a, b) if a == self.a + 1 && self.initiator => {
//self.established = true;
//self.b = b;
//Cmd::Syn(Handshake::Ack(self.a + 1, self.b + 1))
//}
//Handshake::Ack(a, b) if a == self.a + 1 && b == self.b + 1 && !self.initiator => {
//self.established = true;
//Cmd::Ping
//}
//_ => self.restart(),
//}
//}
//}
