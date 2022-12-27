use crate::err::{ThermostazvError, ThermostazvResult};
use crate::sercon::SerialConnection;
use crate::thermostazv::{TCmd, TCmdSender, TWatchReceiver};
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
    while let Ok(cmd) = to_uart_receive.recv().await {
        tracing::debug!("sending {:?} to serial", cmd);
        if let Err(e) = uart_writer.send(cmd).await {
            tracing::error!("serial_writer: I/O error on uart writer: {:?}", e);
        }
    }
}

pub async fn serial_reader(
    mut uart_reader: UartReader,
    to_uart_send: Sender<Cmd>,
    status: Arc<RwLock<Cmd>>,
    to_mqtt_send: Sender<Cmd>,
) -> ThermostazvResult {
    loop {
        if let Some(Ok(cmd)) = uart_reader.next().await {
            tracing::debug!("serial received {:?}", cmd);
            match cmd {
                Cmd::Ping => to_uart_send.send(Cmd::Pong).await?,
                Cmd::Status(r, s) => {
                    let mut st = status.write().map_err(|e| {
                        ThermostazvError::Poison(format!(
                            "Failed to acquire write lock on status in serial_reader {e}"
                        ))
                    })?;
                    *st = Cmd::Status(r, s);
                }
                Cmd::Get | Cmd::Set(_) => tracing::error!("wrong cmd received: {:?}", cmd),
                Cmd::Pong => to_mqtt_send.send(cmd).await?,
            }
        }
    }
}

pub async fn mqtt_receive(
    to_uart_clone: Sender<Cmd>,
    from_mqtt_receive: Receiver<Publish>,
    set_thermostazv: TCmdSender,
    status_clone: Arc<RwLock<Cmd>>,
    to_mqtt_clone: Sender<Cmd>,
) -> ThermostazvResult {
    while let Ok(msg) = from_mqtt_receive.recv().await {
        tracing::info!("mqtt received {:?}", msg);
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
                        "Failed to acquire read lock on status_clone in mqtt_receive {e}"
                    ))
                })?;
                to_mqtt_clone.send(st).await?;
            } else if cmd == "p" {
                to_uart_clone.send(Cmd::Ping).await?;
            }
        } else if topic == "/azv/thermostazv/presence" {
            set_thermostazv
                .send(TCmd::SetPresent(cmd == "présent"))
                .await?;
        } else if topic == "tele/tasmota_43D8FD/SENSOR" {
            let decoded: Result<Value, _> = serde_json::from_slice(&cmd);
            // TODO keep happy path happy
            match decoded {
                Ok(v) => {
                    if let Value::Number(temperature) = &v["SI7021"]["Temperature"] {
                        if let Some(temp) = temperature.as_f64() {
                            set_thermostazv.send(TCmd::Current(temp)).await?;
                        }
                    }
                }
                Err(e) => tracing::error!("Error {} decoding json for '{:?}'", e, &cmd),
            }
        }
    }
    Ok(())
}

pub async fn mqtt_publish(
    to_mqtt_receive: Receiver<Cmd>,
    get_thermostazv: TWatchReceiver,
    client: AsyncClient,
) -> ThermostazvResult {
    while let Ok(cmd) = to_mqtt_receive.recv().await {
        let msg = match cmd {
            Cmd::Get | Cmd::Ping => {
                tracing::error!("wrong command to publish to MQTT");
                None
            }
            Cmd::Set(Relay::Hot) => Some("allumage du chauffe-eau".to_string()),
            Cmd::Set(Relay::Cold) => Some("extinction du chauffe-eau".to_string()),
            Cmd::Pong => Some("pong".to_string()),
            Cmd::Status(relay, sensor) => Some(format!(
                "présent: {}, relay: {:?}, garage: {}",
                get_thermostazv.borrow().present,
                relay,
                match sensor {
                    SensorResult::Ok(s) => format!("{}°C, {}%", s.celsius(), s.rh()),
                    SensorResult::Err(e) => format!("error {e:?}"),
                }
            )),
        };

        if let Some(msg) = msg {
            client
                .publish("/azv/thermostazv/log", QoS::AtLeastOnce, false, msg)
                .await?;
        }
    }
    Ok(())
}

pub async fn influx(
    client: influxdb2::Client,
    get_thermostazv: TWatchReceiver,
    status_infl: Arc<RwLock<Cmd>>,
    infl_buck: &str,
) -> ThermostazvResult {
    loop {
        let mut points = vec![];
        {
            let thermostazv = get_thermostazv.borrow();
            points.push(
                DataPoint::builder("azviot")
                    .tag("device", "thermostazv")
                    .field("relay", thermostazv.hot)
                    .field("absent", !thermostazv.present)
                    .field("targetf", thermostazv.hysteresis())
                    .build()?,
            );
        }
        let mut temperature = None;
        let mut humidity = None;
        {
            let st = status_infl.read().map_err(|e| {
                ThermostazvError::Poison(format!(
                    "Failed to acquire read lock on status_infl in influx {e}"
                ))
            })?;
            if let Cmd::Status(_, SensorResult::Ok(sensor)) = *st {
                temperature = Some(sensor.celsius());
                humidity = Some(sensor.rh());
            }
        }

        if let (Some(temperature), Some(humidity)) = (temperature, humidity) {
            points.push(
                DataPoint::builder("azviot")
                    .tag("device", "thermostazv")
                    .field("Temperature", temperature)
                    .field("Humidity", humidity)
                    .build()?,
            );
        }
        client.write(infl_buck, stream::iter(points)).await?;
        sleep(Duration::from_secs(300)).await;
    }
}
