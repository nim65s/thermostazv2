use std::io::Write;
use std::thread;
use std::time::Duration;

use thermostazv2_lib::{Cmd, SensorResult, HEADER};

struct SerialConnection {
    serial: Box<dyn serialport::SerialPort>,
    header_index: usize,
    buffer: [u8; 32],
    buffer_index: usize,
}

impl SerialConnection {
    pub fn new(serial: Box<dyn serialport::SerialPort>) -> SerialConnection {
        SerialConnection {
            serial,
            header_index: 0,
            buffer: [0; 32],
            buffer_index: 0,
        }
    }

    fn read(&mut self) -> Option<Cmd> {
        let mut dst = [0];
        let _ = self.serial.read(&mut dst).unwrap();
        let byte = dst[0];
        if self.header_index < HEADER.len() {
            if byte == HEADER[self.header_index] {
                self.header_index += 1;
            } else {
                self.header_index = 0;
                eprintln!("wrong header");
            }
        } else {
            if self.header_index == HEADER.len() {
                self.buffer_index = 0;
                self.header_index += 1;
            }
            self.buffer[self.buffer_index] = byte;
            self.buffer_index += 1;
            if self.buffer_index >= 32 {
                self.header_index = 0;
                eprintln!("couldnt parse {:?}", self.buffer);
            } else {
                let config = bincode::config::standard();
                if let Ok((cmd, _)) =
                    bincode::decode_from_slice(&self.buffer[..self.buffer_index], config)
                {
                    {
                        self.buffer_index = 0;
                        self.header_index = 0;
                        return Some(cmd);
                    }
                }
            }
        }
        None
    }

    fn write(&mut self, cmd: &Cmd) {
        if self.serial.write(&HEADER).unwrap() != HEADER.len() {
            eprintln!("couldn't write full header");
        }
        let mut dst = [0; 32];
        let config = bincode::config::standard();
        let size = bincode::encode_into_slice(cmd, &mut dst, config).unwrap();
        if self.serial.write(&dst[..size]).unwrap() != size {
            eprintln!("couldn't write full cmd");
        }
    }
}

fn main() {
    let mut port = serialport::new("/dev/thermostazv2", 2_000_000)
        .open()
        .expect("Failed to open serial port");

    port.set_timeout(Duration::from_millis(60_000)).unwrap();

    let clone = port.try_clone().expect("Failed to clone");

    // writer thread
    thread::spawn(move || {
        let mut serial_connection = SerialConnection::new(clone);
        loop {
            println!("thread loop");
            serial_connection.write(&Cmd::Get);
            thread::sleep(Duration::from_millis(10_000));
        }
    });

    let mut serial_connection = SerialConnection::new(port);

    // reader (main) thread
    loop {
        if let Some(cmd) = serial_connection.read() {
            println!("received {:?}", cmd);
            match cmd {
                Cmd::Ping => println!("pong"),
                Cmd::Sensor(SensorResult::Ok(s)) => println!(" {}%, {}°C", s.rh(), s.celsius()),
                _ => {}
            }
        }
    }
}
