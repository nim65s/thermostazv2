use crossbeam_channel::unbounded;
use rumqttc::mqttbytes::v4::Publish;
use rumqttc::{Client, Event, LastWill, MqttOptions, Packet, QoS};
use serde_json::Value;
use std::env;
use std::sync::{Arc, RwLock};
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

    let thermostazv = Arc::new(RwLock::new(Thermostazv::new()));
    let thermostazv_clone = Arc::clone(&thermostazv);
    let status = Arc::new(RwLock::new(Cmd::Status(
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

    let lwt = LastWill::new("/azv/thermostazv/lwt", "Offline", QoS::AtLeastOnce, false);

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
                //println!("serial received {:?}", cmd);
                match cmd {
                    Cmd::Ping => to_serial_send.send(Cmd::Pong).unwrap(),
                    Cmd::Status(r, s) => {
                        let mut st = status.write().unwrap();
                        *st = Cmd::Status(r, s);
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
        //println!("mqtt received {:?}", msg);
        let topic = msg.topic;
        let cmd = msg.payload;
        if topic == "/azv/thermostazv/cmd" {
            if cmd == "c" {
                to_serial_send_clone.send(Cmd::Set(Relay::Hot)).unwrap();
            } else if cmd == "f" {
                to_serial_send_clone.send(Cmd::Set(Relay::Cold)).unwrap();
            } else if cmd == "s" {
                let st = status_clone.read().unwrap();
                to_mqtt_send_clone.send(*st).unwrap();
            } else if cmd == "p" {
                to_serial_send_clone.send(Cmd::Ping).unwrap();
            }
        } else if topic == "/azv/thermostazv/presence" {
            if cmd == "présent" {
                let mut thermostazv = thermostazv.write().unwrap();
                thermostazv.set_present(true);
            } else if cmd == "absent" {
                let mut thermostazv = thermostazv.write().unwrap();
                thermostazv.set_present(false);
            }
        } else if topic == "tele/tasmota_43D8FD/SENSOR" {
            let decoded: Result<Value, _> = serde_json::from_slice(&cmd);
            match decoded {
                Ok(v) => {
                    if let Value::Number(temperature) = &v["SI7021"]["Temperature"] {
                        let mut thermostazv = thermostazv.write().unwrap();
                        let temp = temperature.as_f64().unwrap();
                        let update = thermostazv.update(temp);
                        let new_relay = if update { Relay::Hot } else { Relay::Cold };
                        to_serial_send_clone.send(Cmd::Set(new_relay)).unwrap();
                        println!("temperature: {} => chauffe: {}", temp, update);
                        if let Cmd::Status(old_relay, _) = *status_clone.read().unwrap() {
                            if old_relay != new_relay {
                                to_mqtt_send_clone.send(Cmd::Set(new_relay)).unwrap();
                            }
                        }
                    }
                }
                Err(e) => eprintln!("Error {} decoding json for '{:?}'", e, &cmd),
            }
        }
    });

    // mqtt publisher thread
    thread::spawn(move || loop {
        let cmd = to_mqtt_receive.recv().unwrap();
        let msg = match cmd {
            Cmd::Get | Cmd::Ping => {
                eprintln!("wrong command to publish to MQTT");
                None
            }
            Cmd::Set(Relay::Hot) => Some("allumage du chauffe-eau".to_string()),
            Cmd::Set(Relay::Cold) => Some("extinction du chauffe-eau".to_string()),
            Cmd::Pong => Some("pong".to_string()),
            Cmd::Status(relay, sensor) => Some(format!(
                "présent: {}, relay: {:?}, garage: {}",
                thermostazv_clone.read().unwrap().is_present(),
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
    eprintln!("no more notifications.");
}
