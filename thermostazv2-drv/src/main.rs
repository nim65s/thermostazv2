//use bincode::{Decode, Encode};
//use rand::random;
use std::io::Write;
use std::thread;
use std::time::Duration;

use thermostazv2_lib::{Cmd, /* Connection, */ Relay, SensorOk, SensorResult, HEADER};

struct SerialConnection {
    serial: Box<dyn serialport::SerialPort>,
    //connection: Connection,
    header_index: usize,
    buffer: [u8; 32],
    buffer_index: usize,
}

impl SerialConnection {
    pub fn new(serial: Box<dyn serialport::SerialPort>) -> SerialConnection {
        //let generate_u8 = Box::new(random::<u8>);
        //let connection = Connection::new(true, generate_u8);
        SerialConnection {
            serial,
            //connection,
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
                //if self.connected() {
                //self.restart();
                //}
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
                //if self.connected() {
                //self.restart();
                //}
            } else {
                let config = bincode::config::standard();
                match bincode::decode_from_slice(&self.buffer[..self.buffer_index], config) {
                    Ok((cmd, _)) => {
                        self.buffer_index = 0;
                        self.header_index = 0;
                        return Some(cmd);
                    }
                    Err(bincode::error::DecodeError::UnexpectedEnd { .. }) => {}
                    _ => {
                        //if self.connected() {
                        //self.restart();
                        //}
                    }
                }
            }
        }
        None
    }

    //fn restart(&mut self) {
    //let cmd = self.connection.restart();
    //self.write(cmd);
    //}

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

    //pub fn connect(&mut self) {
    //while !self.connected() {
    //if let Some(Cmd::Syn(cmd)) = self.read() {
    //let cmd = self.connection.connect(cmd);
    //self.write(cmd);
    //}
    //}
    //}

    //pub fn connected(&self) -> bool {
    //self.connection.connected()
    //}
}

/*
struct SerialCodec {
    serial: Box<dyn serialport::SerialPort>,
}

trait SyncEncoder {
    fn sync_write(&mut self);
}

impl<W: bincode::enc::write::Writer, C: bincode::config::Config> SyncEncoder
    for bincode::enc::EncoderImpl<W, C>
{
    fn sync_write(&mut self) {
        let mut w = self.into_writer();
        w.write(&HEADER).expect("failed to write header");
    }
}

impl SerialCodec {
    fn sync_write(&mut self) {
        self.serial.write(&HEADER).expect("failed to write header");
    }

    fn sync_read(&mut self) {
        let mut buf = [0];
        let mut i = 0;
        loop {
            let _ = self.serial.read(&mut buf).unwrap();
            if buf[0] == HEADER[i] {
                i += 1;
                if i == HEADER.len() {
                    break;
                }
            } else {
                i = 0;
            }
        }
    }
}

impl bincode::enc::write::Writer for SerialCodec {
    fn write(&mut self, bytes: &[u8]) -> Result<(), bincode::error::EncodeError> {
        self.serial
            .write_all(bytes)
            .expect("Failed to write to serial port");
        Ok(())
    }
}

impl bincode::de::read::Reader for SerialCodec {
    fn read(&mut self, bytes: &mut [u8]) -> Result<(), bincode::error::DecodeError> {
        self.serial.read_exact(bytes).unwrap();
        Ok(())
    }
}
*/

fn main() {
    let mut port = serialport::new("/dev/thermostazv2", 2_000_000)
        .open()
        .expect("Failed to open serial port");

    port.set_timeout(Duration::from_millis(60_000)).unwrap();

    let clone = port.try_clone().expect("Failed to clone");

    // writer thread
    thread::spawn(move || {
        //let mut data = Cmd::Status(Relay::Closed, SensorResult::Ok(SensorOk { h: 32, t: 45 }));
        //let config = bincode::config::standard();
        //let serial_encoder = SerialCodec { serial: clone };
        //serial_encoder.sync_write();
        //let mut encoder = bincode::enc::EncoderImpl::new(serial_encoder, config);
        let mut serial_connection = SerialConnection::new(clone);
        loop {
            println!("thread loop");
            //encoder.sync_write();
            //data.encode(&mut encoder).unwrap();
            serial_connection.write(&Cmd::Get);
            thread::sleep(Duration::from_millis(10_000));
        }
    });

    //let config = bincode::config::standard();
    //let mut serial_decoder = SerialCodec { serial: port };
    //serial_decoder.sync_read();
    //let mut decoder = bincode::de::DecoderImpl::new(serial_decoder, config);
    let mut serial_connection = SerialConnection::new(port);

    // reader (main) thread
    loop {
        println!("main loop");
        if let Some(cmd) = serial_connection.read() {
            println!("received {:?}", cmd);
            if let Cmd::Sensor(SensorResult::Ok(s)) = cmd {
                println!(" {}%, {}°C", s.rh(), s.celsius());
            }
        }
    }
}
