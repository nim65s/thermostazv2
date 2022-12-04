use crate::err::{ThermostazvError, ThermostazvResult};
use crate::sercon::SerialConnection;
use crate::thermostazv::Thermostazv;
use async_channel::{Receiver, Sender};
use futures::stream;
use futures::{SinkExt, StreamExt};
use influxdb2::models::DataPoint;
use rumqttc::{AsyncClient, Publish, QoS};
use serde_json::Value;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use thermostazv2_lib::{Cmd, Relay, SensorResult};
use tokio::time::sleep;

type UartWriter =
    stream::SplitSink<tokio_util::codec::Framed<tokio_serial::SerialStream, SerialConnection>, Cmd>;

type UartReader =
    stream::SplitStream<tokio_util::codec::Framed<tokio_serial::SerialStream, SerialConnection>>;

pub async fn serial_writer(to_uart_receive: Receiver<Cmd>, mut uart_writer: UartWriter) {
    println!("starting serial_writer.");
    while let Ok(cmd) = to_uart_receive.recv().await {
        if let Err(e) = uart_writer.send(cmd).await {
            eprintln!("serial_writer: I/O error on uart writer: {:?}", e);
        }
    }
    println!("closing serial_writer.");
}

pub async fn serial_reader(
    mut uart_reader: UartReader,
    to_uart_send: Sender<Cmd>,
    status: Arc<RwLock<Cmd>>,
    to_mqtt_send: Sender<Cmd>,
) -> ThermostazvResult {
    println!("starting serial_reader.");
    loop {
        if let Some(Ok(cmd)) = uart_reader.next().await {
            //println!("serial received {:?}", cmd);
            match cmd {
                Cmd::Ping => to_uart_send.send(Cmd::Pong).await?,
                Cmd::Status(r, s) => {
                    let mut st = status.write().map_err(|e| {
                        ThermostazvError::Poison(format!(
                            "Failed to acquire write lock on status in serial_reader {}",
                            e
                        ))
                    })?;
                    *st = Cmd::Status(r, s);
                }
                Cmd::Get | Cmd::Set(_) => eprintln!("wrong cmd received: {:?}", cmd),
                Cmd::Pong => to_mqtt_send.send(cmd).await?,
            }
        }
    }
    //println!("closing serial_reader.");
}

pub async fn mqtt_receive(
    to_uart_clone: Sender<Cmd>,
    from_mqtt_receive: Receiver<Publish>,
    thermostazv: Arc<RwLock<Thermostazv>>,
    status_clone: Arc<RwLock<Cmd>>,
    to_mqtt_clone: Sender<Cmd>,
) -> ThermostazvResult {
    println!("starting mqtt_receive.");
    while let Ok(msg) = from_mqtt_receive.recv().await {
        //println!("mqtt received {:?}", msg);
        let topic = msg.topic;
        let cmd = msg.payload;
        if topic == "/azv/thermostazv/cmd" {
            if cmd == "c" {
                to_uart_clone.send(Cmd::Set(Relay::Hot)).await?;
            } else if cmd == "f" {
                to_uart_clone.send(Cmd::Set(Relay::Cold)).await?;
            } else if cmd == "s" {
                let st = *status_clone.read().map_err(|e| {
                    ThermostazvError::Poison(format!(
                        "Failed to acquire read lock on status_clone in mqtt_receive {}",
                        e
                    ))
                })?;
                to_mqtt_clone.send(st).await?;
            } else if cmd == "p" {
                to_uart_clone.send(Cmd::Ping).await?;
            }
        } else if topic == "/azv/thermostazv/presence" {
            if cmd == "présent" {
                let mut thermostazv = thermostazv.write().map_err(|e| {
                    ThermostazvError::Poison(format!(
                        "Failed to acquire write lock on thermostazv in mqtt_receive {}",
                        e
                    ))
                })?;
                thermostazv.set_present(true);
            } else if cmd == "absent" {
                let mut thermostazv = thermostazv.write().map_err(|e| {
                    ThermostazvError::Poison(format!(
                        "Failed to acquire write lock on thermostazv in mqtt_receive {}",
                        e
                    ))
                })?;
                thermostazv.set_present(false);
            }
        } else if topic == "tele/tasmota_43D8FD/SENSOR" {
            let decoded: Result<Value, _> = serde_json::from_slice(&cmd);
            match decoded {
                Ok(v) => {
                    if let Value::Number(temperature) = &v["SI7021"]["Temperature"] {
                        let update;
                        if let Some(temp) = temperature.as_f64() {
                            {
                                let mut thermostazv = thermostazv.write().map_err(|e| {
                                    ThermostazvError::Poison(format!(
                        "Failed to acquire write lock on thermostazv in mqtt_receive {}",
                        e
                    ))
                                })?;
                                update = thermostazv.update(temp);
                            }
                            let new_relay = if update { Relay::Hot } else { Relay::Cold };
                            to_uart_clone.send(Cmd::Set(new_relay)).await?;
                            println!("temperature: {} => chauffe: {}", temp, update);
                            let st;
                            {
                                st = *status_clone.read().map_err(|e| {
                                    ThermostazvError::Poison(format!(
                        "Failed to acquire read lock on status_clone in mqtt_receive {}",
                        e
                    ))
                                })?;
                            }

                            if let Cmd::Status(old_relay, _) = st {
                                if old_relay != new_relay {
                                    to_mqtt_clone.send(Cmd::Set(new_relay)).await?;
                                }
                            }
                        }
                    }
                }
                Err(e) => eprintln!("Error {} decoding json for '{:?}'", e, &cmd),
            }
        }
    }
    println!("closing mqtt_receive.");
    Ok(())
}

pub async fn mqtt_publish(
    to_mqtt_receive: Receiver<Cmd>,
    thermostazv_clone: Arc<RwLock<Thermostazv>>,
    client: AsyncClient,
) -> ThermostazvResult {
    println!("starting mqtt_publish.");
    while let Ok(cmd) = to_mqtt_receive.recv().await {
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
                thermostazv_clone
                    .read()
                    .map_err(|e| {
                        ThermostazvError::Poison(format!(
                            "Failed to acquire read lock on thermostazv_clone in mqtt_publish {}",
                            e
                        ))
                    })?
                    .is_present(),
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
                .await?;
        }
    }
    println!("closing mqtt_publish.");
    Ok(())
}

pub async fn influx(
    client: influxdb2::Client,
    thermostazv_infl: Arc<RwLock<Thermostazv>>,
    status_infl: Arc<RwLock<Cmd>>,
    infl_buck: &str,
) -> ThermostazvResult {
    println!("starting influx");
    loop {
        let relay;
        let absent;
        let target;
        let mut temperature = None;
        let mut humidity = None;
        {
            let th = thermostazv_infl.read().map_err(|e| {
                ThermostazvError::Poison(format!(
                    "Failed to acquire read lock on thermostazv_infl in influx {}",
                    e
                ))
            })?;
            absent = !th.is_present();
            relay = th.is_hot();
            target = th.hysteresis();
        }
        {
            let st = status_infl.read().map_err(|e| {
                ThermostazvError::Poison(format!(
                    "Failed to acquire read lock on status_infl in influx {}",
                    e
                ))
            })?;
            if let Cmd::Status(_, SensorResult::Ok(sensor)) = *st {
                temperature = Some(sensor.celsius());
                humidity = Some(sensor.rh());
            }
        }
        let mut points = vec![DataPoint::builder("azviot")
            .tag("device", "thermostazv")
            .field("relay", relay)
            .field("absent", absent)
            .field("targetf", target)
            .build()?];
        if let (Some(temperature), Some(humidity)) = (temperature, humidity) {
            points.push(
                DataPoint::builder("azviot")
                    .tag("device", "thermostazv")
                    .field("Temperature", f64::from(temperature))
                    .field("Humidity", f64::from(humidity))
                    .build()?,
            );
        }
        client.write(infl_buck, stream::iter(points)).await?;
        sleep(Duration::from_secs(300)).await;
    }
    //println!("closing influx");
}
