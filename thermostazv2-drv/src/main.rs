use bincode::{decode_from_slice, encode_into_slice};
use chrono::{Local, Timelike};
use crossbeam_channel::unbounded;
use paho_mqtt as mqtt;
use serde_json::Value;
use std::io::Write;
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use thermostazv2_lib::{Cmd, Relay, SensorErr, SensorResult, HEADER};

struct SerialConnection {
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

    fn read(&mut self) -> Option<Cmd> {
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

    fn write(&mut self, cmd: &Cmd) {
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

struct Thermostazv {
    present: bool,
    state: bool,
}

impl Thermostazv {
    pub fn new() -> Thermostazv {
        Thermostazv {
            present: true,
            state: false,
        }
    }

    fn target(&self) -> f64 {
        if self.present {
            let now = Local::now();
            if 6 <= now.hour() && now.hour() < 23 {
                17.5
            } else {
                17.0
            }
        } else {
            10.0
        }
    }

    fn hysteresis(&self) -> f64 {
        self.target() + if self.state { 0.5 } else { -0.5 }
    }

    pub fn update(&mut self, current_temp: f64) -> bool {
        self.state = current_temp <= self.hysteresis();
        self.state
    }

    pub fn set_present(&mut self, present: bool) {
        self.present = present;
    }

    pub fn is_present(&self) -> bool {
        self.present
    }
}

fn main() {
    let thermostazv = Arc::new(Mutex::new(Thermostazv::new()));
    let thermostazv_clone = Arc::clone(&thermostazv);
    let status = Arc::new(Mutex::new(Cmd::Status(
        Relay::Cold,
        SensorResult::Err(SensorErr::Uninitialized),
    )));
    let status_clone = Arc::clone(&status);
    let mut serial_port = serialport::new("/dev/thermostazv2", 2_000_000)
        .open()
        .expect("Failed to open serial port");

    serial_port
        .set_timeout(Duration::from_millis(60_000))
        .unwrap();

    let serial_clone = serial_port.try_clone().expect("Failed to clone");

    let (to_serial_send, to_serial_receive) = unbounded();
    let to_serial_send_clone = to_serial_send.clone();
    let (to_mqtt_send, to_mqtt_receive) = unbounded();
    let to_mqtt_send_clone = to_mqtt_send.clone();
    //let (from_mqtt_send, from_mqtt_receive) = unbounded();

    // Create the client. Use an ID for a persistent session.
    // A real system should try harder to use a unique ID.
    let create_opts = mqtt::CreateOptionsBuilder::new()
        .server_uri("mqtt://totoro:1883")
        .client_id("thermostazv2")
        .finalize();

    let mqtt_cli = mqtt::Client::new(create_opts).unwrap_or_else(|e| {
        println!("Error creating the client: {:?}", e);
        process::exit(1);
    });

    // Initialize the consumer before connecting
    let mqtt_rx = mqtt_cli.start_consuming();

    // Define the set of options for the connection
    let lwt = mqtt::MessageBuilder::new()
        .topic("/azv/thermostazv2/lwt")
        .payload("Offline")
        .finalize();

    let conn_opts = mqtt::ConnectOptionsBuilder::new()
        .keep_alive_interval(Duration::from_secs(20))
        .clean_session(false)
        .will_message(lwt)
        .finalize();

    let subscriptions = [
        "/azv/thermostazv/cmd",
        "/azv/thermostazv/presence",
        "tele/tasmota_43D8FD/SENSOR",
    ];
    let qos = [1, 1, 1];
    // Make the connection to the broker
    println!("Connecting to the MQTT broker...");
    match mqtt_cli.connect(conn_opts) {
        Ok(rsp) => {
            if let Some(conn_rsp) = rsp.connect_response() {
                println!(
                    "Connected to: '{}' with MQTT version {}",
                    conn_rsp.server_uri, conn_rsp.mqtt_version
                );
                if conn_rsp.session_present {
                    println!("  w/ client session already present on broker.");
                } else {
                    // Register subscriptions on the server
                    println!("Subscribing to topics with requested QoS: {:?}...", qos);

                    mqtt_cli
                        .subscribe_many(&subscriptions, &qos)
                        .and_then(|rsp| {
                            rsp.subscribe_many_response()
                                .ok_or(mqtt::Error::General("Bad response"))
                        })
                        .and_then(|vqos| {
                            println!("QoS granted: {:?}", vqos);
                            Ok(())
                        })
                        .unwrap_or_else(|err| {
                            println!("Error subscribing to topics: {:?}", err);
                            mqtt_cli.disconnect(None).unwrap();
                            process::exit(1);
                        });
                }
            }
        }
        Err(e) => {
            println!("Error connecting to the broker: {:?}", e);
            process::exit(1);
        }
    }

    // serial writer thread
    thread::spawn(move || {
        let mut serial_connection = SerialConnection::new(serial_clone);
        loop {
            let cmd = to_serial_receive.recv().unwrap();
            serial_connection.write(&cmd);
        }
    });

    // serial reader thread
    thread::spawn(move || {
        let mut serial_connection = SerialConnection::new(serial_port);
        loop {
            if let Some(cmd) = serial_connection.read() {
                println!("serial received {:?}", cmd);
                match cmd {
                    Cmd::Ping => to_serial_send.send(Cmd::Pong).unwrap(),
                    Cmd::Status(_, _) => {
                        let mut st = status.lock().unwrap();
                        *st = cmd;
                    }
                    Cmd::Get | Cmd::Set(_) => {
                        eprintln!("wrong cmd received: {:?}", cmd)
                    }
                    Cmd::Pong => to_mqtt_send.send(cmd).unwrap(),
                }
            }
        }
    });

    // mqtt receiver thread
    thread::spawn(move || {
        let msg = mqtt_rx.recv().unwrap();
        println!("mqtt received {:?}", msg);
        if let Some(msg) = msg {
            let topic = msg.topic();
            let cmd = msg.payload_str();
            if topic == "/azv/thermostazv/cmd" {
                if cmd == "c" {
                    to_serial_send_clone.send(Cmd::Set(Relay::Hot)).unwrap();
                } else if cmd == "f" {
                    to_serial_send_clone.send(Cmd::Set(Relay::Cold)).unwrap();
                } else if cmd == "s" {
                    to_mqtt_send_clone
                        .send(*status_clone.lock().unwrap())
                        .unwrap();
                } else if cmd == "p" {
                    to_serial_send_clone.send(Cmd::Ping).unwrap();
                }
            } else if topic == "/azv/thermostazv/presence" {
                if cmd == "présent" {
                    let mut thermostazv = thermostazv.lock().unwrap();
                    thermostazv.set_present(true);
                } else if cmd == "absent" {
                    let mut thermostazv = thermostazv.lock().unwrap();
                    thermostazv.set_present(false);
                }
            } else if topic == "tele/tasmota_43D8FD/SENSOR" {
                let v: Value = serde_json::from_str(&cmd).unwrap();
                if let Value::Number(temperature) = &v["SI7021"]["Temperature"] {
                    let mut thermostazv = thermostazv.lock().unwrap();
                    to_serial_send_clone
                        .send(Cmd::Set(
                            if thermostazv.update(temperature.as_f64().unwrap()) {
                                Relay::Hot
                            } else {
                                Relay::Cold
                            },
                        ))
                        .unwrap();
                }
            }
        }
    });

    // mqtt publish (main) thread
    loop {
        let cmd = to_mqtt_receive.recv().unwrap();
        let msg = match cmd {
            Cmd::Get | Cmd::Ping | Cmd::Set(_) => {
                eprintln!("wrong command to publish to MQTT");
                None
            }
            Cmd::Pong => Some("pong".to_string()),
            Cmd::Status(relay, sensor) => Some(format!(
                "présent: {}, relay: {:?}, garage: {}",
                thermostazv_clone.lock().unwrap().is_present(),
                relay,
                match sensor {
                    SensorResult::Ok(s) => format!("{}°C, {}%", s.celsius(), s.rh()),
                    SensorResult::Err(e) => format!("error {:?}", e),
                }
            )),
        };

        if let Some(msg) = msg {
            let msg = mqtt::MessageBuilder::new()
                .topic("/azv/thermostazv/log")
                .payload(msg)
                .qos(1)
                .finalize();

            if let Err(e) = mqtt_cli.publish(msg) {
                println!("Error sending message: {:?}", e);
            }
        }
    }
}
