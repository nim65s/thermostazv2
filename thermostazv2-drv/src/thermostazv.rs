use crate::err::{ThermostazvError, ThermostazvResult};
use async_channel::Sender;
use chrono::{Local, Timelike};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use thermostazv2_lib::{Cmd, Relay};

#[derive(Serialize, Deserialize, Debug)]
pub enum TCmd {
    SetDay(f64),
    SetNight(f64),
    SetEmpty(f64),
    SetMorning(u32),
    SetEvening(u32),
    SetPresent(bool),
    SetHot(bool),
    Current(f64),
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Thermostazv {
    pub day: f64,
    pub night: f64,
    pub empty: f64,
    pub morning: u32,
    pub evening: u32,
    pub present: bool,
    pub hot: bool,
}

pub type TWatchSender = tokio::sync::watch::Sender<Thermostazv>;
pub type TWatchReceiver = tokio::sync::watch::Receiver<Thermostazv>;
pub type TCmdSender = async_channel::Sender<TCmd>;
pub type TCmdReceiver = async_channel::Receiver<TCmd>;

impl Default for Thermostazv {
    fn default() -> Self {
        Self {
            day: 17.5,
            night: 17.0,
            empty: 10.0,
            morning: 6,
            evening: 22,
            present: true,
            hot: false,
        }
    }
}

fn config_path() -> Box<Path> {
    directories::ProjectDirs::from("", "", "thermostazv2").map_or_else(
        || Path::new("/tmp").into(),
        |proj_dirs| proj_dirs.config_dir().into(),
    )
}

impl Thermostazv {
    pub fn new() -> Result<Self, ThermostazvError> {
        let path = config_path();
        if !path.exists() {
            fs::create_dir_all(&path)?;
        }
        let path = path.join("config.toml");
        Ok(if path.exists() {
            let read = fs::read_to_string(path)?;
            toml::from_str(&read)?
        } else {
            Self::default()
        })
    }

    pub fn save(&self) -> ThermostazvResult {
        let toml = toml::to_string(&self)?;
        fs::write(config_path().join("config.toml"), toml)?;
        Ok(())
    }

    fn target(&self) -> f64 {
        if self.present {
            let now = Local::now();
            if self.morning <= now.hour() && now.hour() < self.evening {
                self.day
            } else {
                self.night
            }
        } else {
            self.empty
        }
    }

    pub fn hysteresis(&self) -> f64 {
        self.target() + if self.hot { 0.5 } else { -0.5 }
    }

    pub fn update(&mut self, current: f64) -> bool {
        let h = self.hysteresis();
        if self.hot == (current <= h) {
            false
        } else {
            self.hot = current <= h;
            tracing::info!("temperature: {} / {} => chauffe: {}", current, h, self.hot);
            true
        }
    }
}

pub struct TManager {
    thermostazv: Thermostazv,
    recv_cmd: TCmdReceiver,
    pub_state: TWatchSender,
    to_uart_send: Sender<Cmd>,
    shutdown_receiver: tokio::sync::watch::Receiver<bool>,
}

impl TManager {
    pub fn new(
        thermostazv: Thermostazv,
        recv_cmd: TCmdReceiver,
        pub_state: TWatchSender,
        to_uart_send: Sender<Cmd>,
        shutdown_receiver: tokio::sync::watch::Receiver<bool>,
    ) -> Self {
        Self {
            thermostazv,
            recv_cmd,
            pub_state,
            to_uart_send,
            shutdown_receiver,
        }
    }

    pub async fn manage(&mut self) -> ThermostazvResult {
        loop {
            tokio::select! {
                _ = self.shutdown_receiver.changed() => return Ok(()),
                req = self.recv_cmd.recv() => if let Ok(req) = req {
                    match req {
                        TCmd::SetDay(val) => self.thermostazv.day = val,
                        TCmd::SetNight(val) => self.thermostazv.night = val,
                        TCmd::SetEmpty(val) => self.thermostazv.empty = val,
                        TCmd::SetMorning(val) => self.thermostazv.morning = val,
                        TCmd::SetEvening(val) => self.thermostazv.evening = val,
                        TCmd::SetPresent(val) => self.thermostazv.present = val,
                        TCmd::SetHot(val) => self.thermostazv.hot = val,
                        TCmd::Current(val) => {
                            if self.thermostazv.update(val) {
                                self.to_uart_send
                                    .send(Cmd::Set(Relay::from(self.thermostazv.hot)))
                                    .await?;
                            }
                        }
                    }
                    self.thermostazv.save()?;
                    self.pub_state.send_if_modified(|old: &mut Thermostazv| {
                        if self.thermostazv == *old {
                            false
                        } else {
                            *old = self.thermostazv.clone();
                            true
                        }
                    });
                }
            }
        }
    }
}
