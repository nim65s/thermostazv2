use thermostazv2_lib::*;
use std::io::Write;
use std::time::Duration;
use std::{io, thread};

fn main() {
    // Open the first serialport available.
    let mut port = serialport::new("/dev/thermostazv2", 2_000_000)
        .open()
        .expect("Failed to open serial port");

    // Clone the port
    let mut clone = port.try_clone().expect("Failed to clone");

    // Send out 4 bytes every second
    thread::spawn(move || {
        let mut data = C::B(B {
            goal: A {
                stop: true,
                pose: 42,
            },
            meas: A {
                stop: false,
                pose: 255,
            },
        });
        let config = bincode::config::standard();
        loop {
            match &mut data {
                C::A(a) => a.stop ^= true,
                C::B(b) => {
                    b.meas.stop ^= true;
                    b.meas.pose = if b.meas.pose > 100 { 42 } else { 255 }
                }
            }
            let mut slice = [0u8; 100];
            let length = bincode::encode_into_slice(&data, &mut slice, config).unwrap();

            clone
                .write_all(&slice[..length])
                .expect("Failed to write to serial port");
            thread::sleep(Duration::from_millis(100));
        }
    });

    // Read the four bytes back from the cloned port
    let mut buffer: [u8; 1] = [0; 1];
    loop {
        match port.read(&mut buffer) {
            Ok(bytes) => {
                if bytes == 1 {
                    println!("Received: {:?}", buffer);
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::TimedOut => (),
            Err(e) => eprintln!("{:?}", e),
        }
    }
}
