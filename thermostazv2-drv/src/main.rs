use anyhow::Context;
use async_channel::unbounded;
use clap::Parser;
use futures::future::try_join_all;
use futures::stream::StreamExt;
use rumqttc::{AsyncClient, LastWill, MqttOptions, QoS};
use std::str::FromStr;
use std::time::Duration;
use thermostazv2_lib::{Cmd, Relay, SensorErr, SensorResult};
use tokio::task;
use tokio::time::sleep;
use tokio_serial::SerialPortBuilderExt;
use tokio_util::codec::Decoder;
use tracing::Level;

mod err;
mod sercon;
mod status;
mod tasks;
mod thermostazv;
use crate::err::ThermostazvResult;
use crate::sercon::SerialConnection;
use crate::status::smanager;
use crate::tasks::{
    influx, main_task, mqtt_connection, mqtt_publish, mqtt_receive, serial_reader, serial_writer,
};
use crate::thermostazv::{TManager, Thermostazv};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, env = "UART_PORT", default_value = "/dev/thermostazv2")]
    uart_port: String,

    #[arg(long, env = "UART_BAUD", default_value_t = 2_000_000)]
    uart_baud: u32,

    #[arg(long, env = "MQTT_HOST", default_value = "totoro")]
    mqtt_host: String,

    #[arg(long, env = "MQTT_PORT", default_value_t = 1883)]
    mqtt_port: u16,

    #[arg(long, env = "MQTT_USER", default_value = "nim")]
    mqtt_user: String,

    #[arg(long, env = "MQTT_PASS")]
    mqtt_pass: String,

    #[arg(long, env = "INFL_BUCK", default_value = "azviot")]
    infl_buck: String,

    #[arg(long, env = "INFL_ORG", default_value = "azviot")]
    infl_org: String,

    #[arg(long, env = "INFL_URL", default_value = "http://localhost:8086")]
    infl_url: String,

    #[arg(long, env = "INFL_TOKEN")]
    infl_token: String,

    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    log_level: String,
}

#[allow(clippy::too_many_lines)]
#[tokio::main]
async fn main() -> ThermostazvResult {
    let args = Args::parse();

    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::from_str(&args.log_level)?)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(false);

    let thermostazv = Thermostazv::new()?;
    let (thermostazv_cmd_send, thermostazv_cmd_receive) = async_channel::unbounded();
    let (thermostazv_watch_send, thermostazv_watch_receive) =
        tokio::sync::watch::channel(thermostazv.clone());

    let status = Cmd::Status(Relay::Cold, SensorResult::Err(SensorErr::Uninitialized));
    let (status_cmd_send, status_cmd_receive) = async_channel::unbounded();
    let (status_watch_send, status_watch_receive) = tokio::sync::watch::channel(status);

    let mut uart_port = tokio_serial::new(args.uart_port, args.uart_baud)
        .open_native_async()
        .context("Failed to open serial port")?;
    uart_port.set_exclusive(false)?;

    let (uart_writer, uart_reader) = SerialConnection::new().framed(uart_port).split();

    let (to_uart_send, to_uart_receive) = unbounded();
    let (to_mqtt_send, to_mqtt_receive) = unbounded();
    let (from_mqtt_send, from_mqtt_receive) = unbounded();
    let to_uart_send2 = to_uart_send.clone();
    let to_uart_send3 = to_uart_send.clone();
    let to_mqtt_send2 = to_mqtt_send.clone();

    let lwt = LastWill::new("/azv/thermostazv/lwt", "Offline", QoS::AtLeastOnce, false);

    let mut mqttoptions = MqttOptions::new("thermostazv2", args.mqtt_host, args.mqtt_port);
    mqttoptions.set_keep_alive(Duration::from_secs(5));
    mqttoptions.set_last_will(lwt);
    mqttoptions.set_credentials(args.mqtt_user, args.mqtt_pass);

    let (client, connection) = AsyncClient::new(mqttoptions, 10);

    let mut tasks = Vec::new();

    tasks.push(task::spawn(serial_writer(
        to_uart_receive,
        uart_writer,
        shutdown_receiver,
    )));
    let shutdown_receiver = shutdown_sender.subscribe();
    tasks.push(task::spawn(serial_reader(
        uart_reader,
        to_uart_send,
        status_cmd_send,
        to_mqtt_send,
        shutdown_receiver,
    )));

    // mqtt receiver task
    let shutdown_receiver = shutdown_sender.subscribe();
    tasks.push(task::spawn(mqtt_receive(
        to_uart_send2,
        from_mqtt_receive,
        thermostazv_cmd_send,
        status_watch_receive,
        to_mqtt_send2,
        shutdown_receiver,
    )));

    client
        .subscribe("/azv/thermostazv/cmd", QoS::AtMostOnce)
        .await?;
    client
        .subscribe("/azv/thermostazv/presence", QoS::AtMostOnce)
        .await?;
    client
        .subscribe("tele/tasmota_43D8FD/SENSOR", QoS::AtMostOnce)
        .await?;
    client
        .publish("/azv/thermostazv/log", QoS::AtLeastOnce, false, "Hi !")
        .await?;

    // mqtt publisher task
    let shutdown_receiver = shutdown_sender.subscribe();
    tasks.push(task::spawn(mqtt_publish(
        to_mqtt_receive,
        thermostazv_watch_receive,
        client,
        shutdown_receiver,
    )));

    let client = influxdb2::Client::new(args.infl_url, args.infl_org, args.infl_token);
    let thermostazv_watch_receive = thermostazv_watch_send.subscribe();
    let status_watch_receive = status_watch_send.subscribe();
    let shutdown_receiver = shutdown_sender.subscribe();
    tasks.push(task::spawn(async move {
        influx(
            client,
            thermostazv_watch_receive,
            status_watch_receive,
            &args.infl_buck,
            shutdown_receiver,
        )
        .await
    }));

    let shutdown_receiver = shutdown_sender.subscribe();
    let mut tmanager = TManager::new(
        thermostazv,
        thermostazv_cmd_receive,
        thermostazv_watch_send,
        to_uart_send3,
        shutdown_receiver,
    );
    tasks.push(task::spawn(async move { tmanager.manage().await }));
    let shutdown_receiver = shutdown_sender.subscribe();
    tasks.push(task::spawn(smanager(
        status_cmd_receive,
        status_watch_send,
        shutdown_receiver,
    )));
    let shutdown_receiver = shutdown_sender.subscribe();
    tasks.push(task::spawn(mqtt_connection(
        connection,
        from_mqtt_send,
        shutdown_receiver,
    )));

    let shutdown_receiver = shutdown_sender.subscribe();
    main_task(&tasks, shutdown_receiver, shutdown_sender).await;

    sleep(Duration::from_secs(3)).await;
    try_join_all(tasks).await?;

    Ok(())
}
