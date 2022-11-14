use chrono::{Local, Timelike};

pub struct Thermostazv {
    present: bool,
    state: bool,
}

impl Thermostazv {
    pub fn new() -> Thermostazv {
        Thermostazv {
            present: true,
            state: false,
        }
    }

    fn target(&self) -> f64 {
        if self.present {
            let now = Local::now();
            if 6 <= now.hour() && now.hour() < 23 {
                17.5
            } else {
                17.0
            }
        } else {
            10.0
        }
    }

    fn hysteresis(&self) -> f64 {
        self.target() + if self.state { 0.5 } else { -0.5 }
    }

    pub fn update(&mut self, current_temp: f64) -> bool {
        self.state = current_temp <= self.hysteresis();
        self.state
    }

    pub fn set_present(&mut self, present: bool) {
        self.present = present;
    }

    pub fn is_present(&self) -> bool {
        self.present
    }
}
