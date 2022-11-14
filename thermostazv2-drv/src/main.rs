use crossbeam_channel::unbounded;
use paho_mqtt as mqtt;
use serde_json::Value;
use std::env;
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use thermostazv2_lib::{Cmd, Relay, SensorErr, SensorResult};

mod sercon;
mod thermostazv;
use crate::sercon::SerialConnection;
use crate::thermostazv::Thermostazv;

fn main() {
    let ser_port = env::var("SER_PORT").unwrap_or("/dev/thermostazv2".to_string());
    let mqtt_host = env::var("MQTT_HOST").unwrap_or("mqtt://totoro:1883".to_string());
    let mqtt_user = env::var("MQTT_USER").unwrap_or("nim".to_string());
    let mqtt_pass = env::var("MQTT_PASS").unwrap_or("".to_string());

    let thermostazv = Arc::new(Mutex::new(Thermostazv::new()));
    let thermostazv_clone = Arc::clone(&thermostazv);
    let status = Arc::new(Mutex::new(Cmd::Status(
        Relay::Cold,
        SensorResult::Err(SensorErr::Uninitialized),
    )));
    let status_clone = Arc::clone(&status);
    let mut serial_port = serialport::new(ser_port, 2_000_000)
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

    // Create the client. Use an ID for a persistent session.
    // A real system should try harder to use a unique ID.
    let create_opts = mqtt::CreateOptionsBuilder::new()
        .server_uri(mqtt_host)
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
        .user_name(mqtt_user)
        .password(mqtt_pass)
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
