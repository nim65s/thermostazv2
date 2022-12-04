//! copy paste from <https://github.com/nim65s/aht20/blob/master/src/lib.rs> to remove delays for
//! rtic

use {
    bitflags::bitflags,
    crc::{Algorithm, Crc},
    embedded_hal::blocking::i2c::{Write, WriteRead},
};

const I2C_ADDRESS: u8 = 0x38;
const CRC_ALGO: Algorithm<u8> = Algorithm {
    width: 16,
    poly: 0b11_0001,
    init: 0xFF,
    refin: false,
    refout: false,
    xorout: 0,
    check: 0xf7,
    residue: 0,
};

bitflags! {
    struct StatusFlags: u8 {
        const BUSY = (1 << 7);
        const MODE = ((1 << 6) | (1 << 5));
        const CRC = (1 << 4);
        const CALIBRATION_ENABLE = (1 << 3);
        const FIFO_ENABLE = (1 << 2);
        const FIFO_FULL = (1 << 1);
        const FIFO_EMPTY = (1 << 0);
    }
}

/// AHT20 Error.
#[derive(Debug, Copy, Clone)]
pub enum Error<E> {
    /// Device is not calibrated.
    Uncalibrated,
    /// Underlying bus error.
    Bus(E),
    /// Checksum mismatch.
    Checksum,
}

impl<E> core::convert::From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::Bus(e)
    }
}

/// Humidity reading from AHT20.
pub struct Humidity {
    h: u32,
}

impl Humidity {
    /// Humidity converted to Relative Humidity %.
    pub fn rh(&self) -> f64 {
        100.0 * f64::from(self.h) / f64::from(1 << 20)
    }

    /// Raw humidity reading.
    pub const fn raw(&self) -> u32 {
        self.h
    }
}

/// Temperature reading from AHT20.
pub struct Temperature {
    t: u32,
}

impl Temperature {
    /// Temperature converted to Celsius.
    pub fn celsius(&self) -> f64 {
        200.0 * f64::from(self.t) / f64::from(1 << 20) - 50.0
    }

    /// Raw temperature reading.
    pub const fn raw(&self) -> u32 {
        self.t
    }
}

/// AHT20 driver.
pub struct Aht20Rtic<I2C> {
    i2c: I2C,
}

impl<I2C, E> Aht20Rtic<I2C>
where
    I2C: WriteRead<Error = E> + Write<Error = E>,
{
    /// Creates a new AHT20 device from an I2C peripheral
    pub fn new(i2c: I2C) -> Result<Self, Error<E>> {
        let mut dev = Self { i2c };

        dev.calibrate()?;

        Ok(dev)
    }

    /// Gets the sensor status.
    fn status(&mut self) -> Result<StatusFlags, E> {
        let buf = &mut [0u8; 1];
        self.i2c.write_read(I2C_ADDRESS, &[0u8], buf)?;

        Ok(StatusFlags { bits: buf[0] })
    }

    /// Self-calibrate the sensor.
    pub fn calibrate(&mut self) -> Result<(), Error<E>> {
        // Send calibrate command
        self.i2c.write(I2C_ADDRESS, &[0xE1, 0x08, 0x00])?;
        Ok(())
    }

    pub fn busy(&mut self) -> Result<bool, Error<E>> {
        Ok(self.status()?.contains(StatusFlags::BUSY))
    }

    // Confirm sensor is calibrated
    pub fn calibrated(&mut self) -> Result<(), Error<E>> {
        if !self.status()?.contains(StatusFlags::CALIBRATION_ENABLE) {
            return Err(Error::Uncalibrated);
        }

        Ok(())
    }

    /// Soft resets the sensor.
    pub fn reset(&mut self) -> Result<(), E> {
        // Send soft reset command
        self.i2c.write(I2C_ADDRESS, &[0xBA])?;
        Ok(())
    }

    /// Reads humidity and temperature.
    pub fn start_read(&mut self) -> Result<(), Error<E>> {
        // Send trigger measurement command
        self.i2c.write(I2C_ADDRESS, &[0xAC, 0x33, 0x00])?;
        Ok(())
    }

    pub fn end_read(&mut self) -> Result<(Humidity, Temperature), Error<E>> {
        let crc = Crc::<u8>::new(&CRC_ALGO);
        let mut digest = crc.digest();

        // Read in sensor data
        let buf = &mut [0u8; 7];
        self.i2c.write_read(I2C_ADDRESS, &[0u8], buf)?;

        // Check for CRC mismatch
        digest.update(&buf[..=5]);
        if digest.finalize() != buf[6] {
            return Err(Error::Checksum);
        };

        // Check calibration
        let status = StatusFlags { bits: buf[0] };
        if !status.contains(StatusFlags::CALIBRATION_ENABLE) {
            return Err(Error::Uncalibrated);
        }

        // Extract humitidy and temperature values from data
        let buf: [u32; 7] = [
            u32::from(buf[0]),
            u32::from(buf[1]),
            u32::from(buf[2]),
            u32::from(buf[3]),
            u32::from(buf[4]),
            u32::from(buf[5]),
            u32::from(buf[6]),
        ];
        let hum = (buf[1] << 12) | (buf[2] << 4) | (buf[3] >> 4);
        let temp = ((buf[3] & 0x0f) << 16) | (buf[4] << 8) | buf[5];

        Ok((Humidity { h: hum }, Temperature { t: temp }))
    }
}
