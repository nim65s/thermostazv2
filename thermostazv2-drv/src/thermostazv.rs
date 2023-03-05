use chrono::{Local, Timelike};

#[derive(Debug)]
pub struct Thermostazv {
    present: bool,
    hot: bool,
}

impl Thermostazv {
    pub const fn new() -> Self {
        Self {
            present: true,
            hot: false,
        }
    }

    fn target(&self) -> f64 {
        if self.present {
            /*
            let now = Local::now();
            if 6 <= now.hour() && now.hour() < 23 {
                17.5
            } else {
                17.0
            }
            */
            17.5
        } else {
            10.0
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
    }

    pub const fn is_present(&self) -> bool {
        self.present
    }

    pub const fn is_hot(&self) -> bool {
        self.hot
    }
}
