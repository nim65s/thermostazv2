use bincode::{Decode, Encode};
use std::io::Write;
use std::thread;
use std::time::Duration;

use thermostazv2_lib::{Cmd, Relay, SensorOk, SensorResult};

struct SerialCodec {
    serial: Box<dyn serialport::SerialPort>,
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

fn main() {
    let port = serialport::new("/dev/thermostazv2", 2_000_000)
        .open()
        .expect("Failed to open serial port");

    let clone = port.try_clone().expect("Failed to clone");

    // writer thread
    thread::spawn(move || {
        let mut data = Cmd::Status(Relay::Closed, SensorResult::Ok(SensorOk { h: 32, t: 45 }));
        let config = bincode::config::standard();
        let serial_encoder = SerialCodec { serial: clone };
        let mut encoder = bincode::enc::EncoderImpl::new(serial_encoder, config);
        loop {
            if let Cmd::Status(r, s) = data {
                data = Cmd::Status(
                    if r == Relay::Closed {
                        Relay::Open
                    } else {
                        Relay::Closed
                    },
                    s,
                );
            }
            data.encode(&mut encoder).unwrap();
            thread::sleep(Duration::from_millis(100));
        }
    });

    let config = bincode::config::standard();
    let serial_decoder = SerialCodec { serial: port };
    let mut decoder = bincode::de::DecoderImpl::new(serial_decoder, config);

    // reader (main) thread
    loop {
        let data = Cmd::decode(&mut decoder).unwrap();
        println!("received {:?}", data);
    }
}
