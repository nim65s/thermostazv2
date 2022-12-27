use chrono::{Local, Timelike};

use crate::err::ThermostazvError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Serialize, Deserialize, Debug)]
pub struct Thermostazv {
    pub day: f64,
    pub night: f64,
    pub empty: f64,
    pub morning: u32,
    pub evening: u32,
    pub present: bool,
    pub hot: bool,
}

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

    pub fn save(&self) -> Result<(), ThermostazvError> {
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
        self.hot = current <= h;
        tracing::info!("temperature: {} / {} => chauffe: {}", current, h, self.hot);
        self.hot
    }

    pub fn set_present(&mut self, present: bool) {
        self.present = present;
        self.save();
    }

    pub fn set_day_temp(&mut self, val: f64) {
        self.day = val;
        self.save();
    }

    pub fn set_night_temp(&mut self, val: f64) {
        self.night = val;
        self.save();
    }

    pub fn set_empty_temp(&mut self, val: f64) {
        self.empty = val;
        self.save();
    }

    pub fn set_morning_hour(&mut self, val: u32) {
        self.morning = val;
        self.save();
    }

    pub fn set_evening_hour(&mut self, val: u32) {
        self.evening = val;
        self.save();
    }

    pub const fn is_present(&self) -> bool {
        self.present
    }

    pub const fn is_hot(&self) -> bool {
        self.hot
    }
}
