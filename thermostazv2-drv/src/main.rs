use crossbeam_channel::unbounded;
use rumqttc::mqttbytes::v4::Publish;
use rumqttc::{Client, Event, LastWill, MqttOptions, Packet, QoS};
use serde_json::Value;
use std::env;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use thermostazv2_lib::{Cmd, Relay, SensorErr, SensorResult};

mod sercon;
mod thermostazv;
use crate::sercon::SerialConnection;
use crate::thermostazv::Thermostazv;

fn main() {
    let uart_port = env::var("UART_PORT").unwrap_or_else(|_| "/dev/thermostazv2".into());
    let mqtt_host = env::var("MQTT_HOST").unwrap_or_else(|_| "totoro".into());
    let mqtt_user = env::var("MQTT_USER").unwrap_or_else(|_| "nim".into());
    let mqtt_pass = env::var("MQTT_PASS").unwrap_or_else(|_| "".into());

    let thermostazv = Arc::new(Mutex::new(Thermostazv::new()));
    let thermostazv_clone = Arc::clone(&thermostazv);
    let status = Arc::new(Mutex::new(Cmd::Status(
        Relay::Cold,
        SensorResult::Err(SensorErr::Uninitialized),
    )));
    let status_clone = Arc::clone(&status);
    let mut serial_port = serialport::new(uart_port, 2_000_000)
        .open()
        .expect("Failed to open serial port");

    serial_port
        .set_timeout(Duration::from_millis(60_000))
        .unwrap();

    let serial_clone = serial_port.try_clone().expect("Failed to clone");

    let (to_serial_send, to_serial_receive) = unbounded();
    let (to_mqtt_send, to_mqtt_receive) = unbounded();
    let (from_mqtt_send, from_mqtt_receive) = unbounded();
    let to_serial_send_clone = to_serial_send.clone();
    let to_mqtt_send_clone = to_mqtt_send.clone();

    let lwt = LastWill::new("/azv/thermostazv2/lwt", "Offline", QoS::AtLeastOnce, false);

    let mut mqttoptions = MqttOptions::new("thermostazv2", mqtt_host, 1883);
    mqttoptions.set_keep_alive(Duration::from_secs(5));
    mqttoptions.set_last_will(lwt);
    mqttoptions.set_credentials(mqtt_user, mqtt_pass);

    let (mut client, mut connection) = Client::new(mqttoptions, 10);

    client
        .subscribe("/azv/thermostazv/cmd", QoS::AtMostOnce)
        .unwrap();
    client
        .subscribe("/azv/thermostazv/presence", QoS::AtMostOnce)
        .unwrap();
    client
        .subscribe("tele/tasmota_43D8FD/SENSOR", QoS::AtMostOnce)
        .unwrap();

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
    thread::spawn(move || loop {
        let msg: Publish = from_mqtt_receive.recv().unwrap();
        println!("mqtt received {:?}", msg);
        let topic = msg.topic;
        let cmd = msg.payload;
        if topic == "/azv/thermostazv/cmd" {
            if cmd == "c" {
                to_serial_send_clone.send(Cmd::Set(Relay::Hot)).unwrap();
            } else if cmd == "f" {
                to_serial_send_clone.send(Cmd::Set(Relay::Cold)).unwrap();
            } else if cmd == "s" {
                let st = status_clone.lock().unwrap();
                to_mqtt_send_clone.send(*st).unwrap();
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
            let v: Value = serde_json::from_str(&cmd.escape_ascii().to_string()).unwrap();
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
    });

    // mqtt publisher thread
    thread::spawn(move || {
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
            client
                .publish("/azv/thermostazv/log", QoS::AtLeastOnce, false, msg)
                .unwrap();
        }
    });

    // mqtt publish (main) thread
    for notification in connection.iter() {
        match notification {
            Ok(Event::Incoming(Packet::Publish(p))) => from_mqtt_send.send(p).unwrap(),
            Err(n) => eprintln!("Err:  {:?}", n),
            Ok(_) => {}
        }
    }
    println!("no more notifications.");
}
