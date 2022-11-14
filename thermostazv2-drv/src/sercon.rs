use bincode::{decode_from_slice, encode_into_slice};
use std::io::Write;
use thermostazv2_lib::{Cmd, HEADER};

pub struct SerialConnection {
    serial: Box<dyn serialport::SerialPort>,
    header_index: usize,
    buffer: [u8; 32],
    buffer_index: usize,
    buffer_size: usize,
}

impl SerialConnection {
    pub fn new(serial: Box<dyn serialport::SerialPort>) -> SerialConnection {
        SerialConnection {
            serial,
            header_index: 0,
            buffer: [0; 32],
            buffer_index: 0,
            buffer_size: 0,
        }
    }

    pub fn read(&mut self) -> Option<Cmd> {
        let mut ret = None;
        let mut dst = [0];
        let _ = self.serial.read(&mut dst).unwrap();
        let byte = dst[0];
        if self.header_index < HEADER.len() {
            if byte == HEADER[self.header_index] {
                self.header_index += 1;
            } else {
                eprintln!("wrong header {}: {}", self.header_index, byte);
                self.header_index = 0;
                self.buffer_index = 0;
                self.buffer_size = 0;
            }
        } else if self.header_index == HEADER.len() {
            self.buffer_index = 0;
            self.header_index += 1;
            self.buffer_size = byte.into();
        } else {
            self.buffer[self.buffer_index] = byte;
            self.buffer_index += 1;
            if self.buffer_index == self.buffer_size {
                let config = bincode::config::standard();
                if let Ok((cmd, _)) = decode_from_slice(&self.buffer[..self.buffer_size], config) {
                    ret = Some(cmd);
                } else {
                    eprintln!("couldn't decode {:?}", &self.buffer[..self.buffer_size]);
                }
                self.header_index = 0;
                self.buffer_index = 0;
                self.buffer_size = 0;
            }
        }
        ret
    }

    pub fn write(&mut self, cmd: &Cmd) {
        if self.serial.write(&HEADER).unwrap() != HEADER.len() {
            eprintln!("couldn't write full header");
        }
        let mut dst = [0; 32];
        let config = bincode::config::standard();
        let size = encode_into_slice(cmd, &mut dst, config).unwrap();
        if self.serial.write(&[size.try_into().unwrap()]).unwrap() != 1 {
            eprintln!("couldn't write packet length");
        }
        if self.serial.write(&dst[..size]).unwrap() != size {
            eprintln!("couldn't write full cmd");
        }
    }
}
