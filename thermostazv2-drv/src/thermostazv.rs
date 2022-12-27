use chrono::{Local, Timelike};

use crate::err::ThermostazvError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

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

    pub fn save(&self, pub_state: &TWatchSender) -> Result<(), ThermostazvError> {
        pub_state.send_if_modified(|old: &mut Self| {
            if self == old {
                false
            } else {
                *old = self.clone();
                true
            }
        });
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

    pub fn update(&mut self, current: f64) {
        let h = self.hysteresis();
        self.hot = current <= h;
        tracing::info!("temperature: {} / {} => chauffe: {}", current, h, self.hot);
        // TODO: if self.hot changed, notify serial and mqtt
    }

    pub async fn manager(
        &mut self,
        recv_cmd: TCmdReceiver,
        pub_state: TWatchSender,
    ) -> Result<(), ThermostazvError> {
        while let Ok(req) = recv_cmd.recv().await {
            match req {
                TCmd::SetDay(val) => self.day = val,
                TCmd::SetNight(val) => self.night = val,
                TCmd::SetEmpty(val) => self.empty = val,
                TCmd::SetMorning(val) => self.morning = val,
                TCmd::SetEvening(val) => self.evening = val,
                TCmd::SetPresent(val) => self.present = val,
                TCmd::SetHot(val) => self.hot = val,
                TCmd::Current(val) => self.update(val),
            }
            self.save(&pub_state)?;
        }
        Ok(())
    }
}
