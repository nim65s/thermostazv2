use async_channel::unbounded;
use futures::{stream::StreamExt, SinkExt};
use rumqttc::mqttbytes::v4::Publish;
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS};
use serde_json::Value;
use std::env;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use thermostazv2_lib::{Cmd, Relay, SensorErr, SensorResult};
use tokio::task;
use tokio_serial::SerialPortBuilderExt;
use tokio_util::codec::Decoder;

mod sercon;
mod thermostazv;
use crate::sercon::SerialConnection;
use crate::thermostazv::Thermostazv;

#[tokio::main]
async fn main() {
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

    let mut uart_port = tokio_serial::new(uart_port, 2_000_000)
        .open_native_async()
        .expect("Failed to open serial port");
    uart_port.set_exclusive(false).unwrap();

    let (mut uart_writer, mut uart_reader) = SerialConnection::new().framed(uart_port).split();

    let (to_uart_send, to_uart_receive) = unbounded();
    let (to_mqtt_send, to_mqtt_receive) = unbounded();
    let (from_mqtt_send, from_mqtt_receive) = unbounded();
    let to_uart_send_clone = to_uart_send.clone();
    let to_mqtt_send_clone = to_mqtt_send.clone();

    let lwt = LastWill::new("/azv/thermostazv/lwt", "Offline", QoS::AtLeastOnce, false);

    let mut mqttoptions = MqttOptions::new("thermostazv2", mqtt_host, 1883);
    mqttoptions.set_keep_alive(Duration::from_secs(5));
    mqttoptions.set_last_will(lwt);
    mqttoptions.set_credentials(mqtt_user, mqtt_pass);

    let (client, mut connection) = AsyncClient::new(mqttoptions, 10);

    // serial writer task
    task::spawn(async move {
        loop {
            let cmd = to_uart_receive.recv().await.unwrap();
            uart_writer.send(cmd).await.unwrap();
        }
    });

    // serial reader task
    task::spawn(async move {
        loop {
            if let Some(Ok(cmd)) = uart_reader.next().await {
                //println!("serial received {:?}", cmd);
                match cmd {
                    Cmd::Ping => to_uart_send.send(Cmd::Pong).await.unwrap(),
                    Cmd::Status(r, s) => {
                        let mut st = status.write().unwrap();
                        *st = Cmd::Status(r, s);
                    }
                    Cmd::Get | Cmd::Set(_) => {
                        eprintln!("wrong cmd received: {:?}", cmd)
                    }
                    Cmd::Pong => to_mqtt_send.send(cmd).await.unwrap(),
                }
            }
        }
    });

    // mqtt receiver task
    task::spawn(async move {
        loop {
            let msg: Publish = from_mqtt_receive.recv().await.unwrap();
            //println!("mqtt received {:?}", msg);
            let topic = msg.topic;
            let cmd = msg.payload;
            if topic == "/azv/thermostazv/cmd" {
                if cmd == "c" {
                    to_uart_send_clone.send(Cmd::Set(Relay::Hot)).await.unwrap();
                } else if cmd == "f" {
                    to_uart_send_clone
                        .send(Cmd::Set(Relay::Cold))
                        .await
                        .unwrap();
                } else if cmd == "s" {
                    let st_clone;
                    {
                        st_clone = *status_clone.read().unwrap();
                    }
                    to_mqtt_send_clone.send(st_clone).await.unwrap();
                } else if cmd == "p" {
                    to_uart_send_clone.send(Cmd::Ping).await.unwrap();
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
                            let update;
                            let temp = temperature.as_f64().unwrap();
                            {
                                let mut thermostazv = thermostazv.write().unwrap();
                                update = thermostazv.update(temp);
                            }
                            let new_relay = if update { Relay::Hot } else { Relay::Cold };
                            to_uart_send_clone.send(Cmd::Set(new_relay)).await.unwrap();
                            println!("temperature: {} => chauffe: {}", temp, update);
                            let st;
                            {
                                st = *status_clone.read().unwrap();
                            }

                            if let Cmd::Status(old_relay, _) = st {
                                if old_relay != new_relay {
                                    to_mqtt_send_clone.send(Cmd::Set(new_relay)).await.unwrap();
                                }
                            }
                        }
                    }
                    Err(e) => eprintln!("Error {} decoding json for '{:?}'", e, &cmd),
                }
            }
        }
    });

    client
        .subscribe("/azv/thermostazv/cmd", QoS::AtMostOnce)
        .await
        .unwrap();
    client
        .subscribe("/azv/thermostazv/presence", QoS::AtMostOnce)
        .await
        .unwrap();
    client
        .subscribe("tele/tasmota_43D8FD/SENSOR", QoS::AtMostOnce)
        .await
        .unwrap();

    // mqtt publisher task
    task::spawn(async move {
        loop {
            let cmd = to_mqtt_receive.recv().await.unwrap();
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
                    .await
                    .unwrap();
            }
        }
    });

    // mqtt publish (main) task
    loop {
        match connection.poll().await {
            Ok(Event::Incoming(Packet::Publish(p))) => from_mqtt_send.send(p).await.unwrap(),
            Err(n) => eprintln!("incoming mqtt packet Err:  {:?}", n),
            Ok(_) => {}
        }
    }
}
