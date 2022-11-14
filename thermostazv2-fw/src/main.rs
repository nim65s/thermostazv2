//! CDC-ACM serial port example using cortex-m-rtic.
//! Target board: Blue Pill
//! with bincode & rtt
#![no_main]
#![no_std]
#![allow(non_snake_case)]

use panic_rtt_target as _;

mod aht20rtic;

#[rtic::app(device = stm32f1xx_hal::pac, peripherals = true, dispatchers = [SPI1, SPI2, SPI3, ADC1_2, ADC3, CAN_RX1, CAN_SCE])]
mod app {
    use crate::aht20rtic::{Aht20Rtic, Error};
    use bincode::{decode_from_slice, encode_into_slice};
    use cortex_m::asm::delay;
    use rtt_target::{rprintln, rtt_init_print};
    use stm32f1xx_hal::gpio::PinState;
    use stm32f1xx_hal::gpio::{Alternate, OpenDrain, Output, PushPull, PB6, PB7, PB8, PC13};
    use stm32f1xx_hal::i2c::{BlockingI2c, DutyCycle, Mode};
    use stm32f1xx_hal::pac::I2C1;
    use stm32f1xx_hal::prelude::*;
    use stm32f1xx_hal::usb::{Peripheral, UsbBus, UsbBusType};
    use systick_monotonic::{fugit::Duration, Systick};
    use thermostazv2_lib::*;
    use usb_device::prelude::*;

    type I2c = BlockingI2c<I2C1, (PB6<Alternate<OpenDrain>>, PB7<Alternate<OpenDrain>>)>;

    #[shared]
    struct Shared {
        usb_dev: UsbDevice<'static, UsbBusType>,
        serial: usbd_serial::SerialPort<'static, UsbBusType>,
        data: Cmd,
        aht20rtic: Aht20Rtic<I2c>,
        relay: PB8<Output<PushPull>>,
        sensor: SensorResult,
    }

    #[local]
    struct Local {
        led: PC13<Output<PushPull>>,
        state: bool,
        header_index: usize,
        buffer: [u8; 32],
        buffer_index: usize,
        buffer_size: usize,
    }

    #[monotonic(binds = SysTick, default = true)]
    type MonoTimer = Systick<1000>;

    #[init]
    fn init(cx: init::Context) -> (Shared, Local, init::Monotonics) {
        rtt_init_print!();
        rprintln!("init start");
        static mut USB_BUS: Option<usb_device::bus::UsbBusAllocator<UsbBusType>> = None;

        let mut flash = cx.device.FLASH.constrain();
        let rcc = cx.device.RCC.constrain();
        let mono = Systick::new(cx.core.SYST, 36_000_000);

        let clocks = rcc
            .cfgr
            .use_hse(8.MHz())
            .sysclk(48.MHz())
            .pclk1(24.MHz())
            .freeze(&mut flash.acr);

        assert!(clocks.usbclk_valid());

        let mut gpioa = cx.device.GPIOA.split();
        let mut gpiob = cx.device.GPIOB.split();
        let mut gpioc = cx.device.GPIOC.split();

        // BluePill board has a pull-up resistor on the D+ line.
        // Pull the D+ pin down to send a RESET condition to the USB bus.
        // This forced reset is needed only for development, without it host
        // will not reset your device when you upload new firmware.
        let usb_dp = gpioa
            .pa12
            .into_push_pull_output_with_state(&mut gpioa.crh, PinState::Low);
        delay(clocks.sysclk().raw() / 100);

        let usb_dm = gpioa.pa11;
        let usb_dp = usb_dp.into_floating_input(&mut gpioa.crh);

        let usb = Peripheral {
            usb: cx.device.USB,
            pin_dm: usb_dm,
            pin_dp: usb_dp,
        };

        unsafe {
            USB_BUS.replace(UsbBus::new(usb));
        }

        let serial = usbd_serial::SerialPort::new(unsafe { USB_BUS.as_ref().unwrap() });

        let usb_dev = UsbDeviceBuilder::new(
            unsafe { USB_BUS.as_ref().unwrap() },
            UsbVidPid(0x6565, 0x0003),
        )
        .manufacturer("Nim")
        .product("thermostazv2")
        .serial_number("0001")
        .device_class(usbd_serial::USB_CLASS_CDC)
        .build();

        let scl = gpiob.pb6.into_alternate_open_drain(&mut gpiob.crl);
        let sda = gpiob.pb7.into_alternate_open_drain(&mut gpiob.crl);
        let mut afio = cx.device.AFIO.constrain();

        let i2c = BlockingI2c::i2c1(
            cx.device.I2C1,
            (scl, sda),
            &mut afio.mapr,
            Mode::Fast {
                frequency: 400.kHz(),
                duty_cycle: DutyCycle::Ratio16to9,
            },
            clocks,
            1000,
            10,
            1000,
            1000,
        );

        let aht20rtic = Aht20Rtic::new(i2c);
        if aht20rtic.is_err() {
            rprintln!("ahrt20 err");
        }
        let aht20rtic = aht20rtic.unwrap();

        let data = Cmd::Status(Relay::Close, SensorResult::Ok(SensorOk { h: 32, t: 45 }));
        let led = gpioc
            .pc13
            .into_push_pull_output_with_state(&mut gpioc.crh, PinState::Low);
        blink::spawn_after(Duration::<u64, 1, 1000>::from_ticks(1000)).unwrap();
        wait_calibrate::spawn_after(Duration::<u64, 1, 1000>::from_ticks(20)).unwrap();
        rprintln!("init end");

        gpiob
            .pb9
            .into_push_pull_output_with_state(&mut gpiob.crh, PinState::Low);
        let relay = gpiob
            .pb8
            .into_push_pull_output_with_state(&mut gpiob.crh, PinState::Low);

        let sensor = SensorResult::Err(SensorErr::Uninitialized);

        (
            Shared {
                usb_dev,
                serial,
                data,
                aht20rtic,
                relay,
                sensor,
            },
            Local {
                led,
                state: false,
                header_index: 0,
                buffer: [0; 32],
                buffer_index: 0,
                buffer_size: 0,
            },
            init::Monotonics(mono),
        )
    }

    #[task(capacity = 3, local = [header_index, buffer, buffer_index, buffer_size])]
    fn decode(cx: decode::Context, buf: [u8; 32], count: usize) {
        let header_index = cx.local.header_index;
        let buffer = cx.local.buffer;
        let buffer_index = cx.local.buffer_index;
        let buffer_size = cx.local.buffer_size;

        for &byte in &buf[..count] {
            if *header_index < HEADER.len() {
                if byte == HEADER[*header_index] {
                    *header_index += 1;
                } else {
                    rprintln!("wrong header {}: {}", *header_index, byte);
                    *header_index = 0;
                    *buffer_index = 0;
                    *buffer_size = 0;
                }
            } else if *header_index == HEADER.len() {
                *buffer_index = 0;
                *header_index += 1;
                *buffer_size = byte.into();
            } else {
                buffer[*buffer_index] = byte;
                *buffer_index += 1;
                if *buffer_index == *buffer_size {
                    let conf = bincode::config::standard();
                    if let Ok((cmd, _)) = decode_from_slice::<Cmd, bincode::config::Configuration>(
                        &buffer[..*buffer_size],
                        conf,
                    ) {
                        //rprintln!("decode {} / {}: {:?}", size, count, cmd);
                        rprintln!("received {:?}", cmd);
                        match cmd {
                            Cmd::Get => send_status::spawn().unwrap(),
                            Cmd::Ping => send::spawn(Cmd::Pong).unwrap(),
                            Cmd::Set(r) => set_relay::spawn(r).unwrap(),
                            Cmd::Sensor(_) | Cmd::Status(_, _) => {
                                rprintln!("wrong cmd received: {:?}", cmd)
                            }
                            Cmd::Pong => rprintln!("pong"),
                        }
                    } else {
                        rprintln!("Couldn't decode {:?}", &buffer[..*buffer_size]);
                    }
                    *header_index = 0;
                    *buffer_index = 0;
                    *buffer_size = 0;
                }
            }
        }
    }

    #[task(binds = USB_HP_CAN_TX, shared = [usb_dev, serial])]
    fn usb_tx(cx: usb_tx::Context) {
        let mut usb_dev = cx.shared.usb_dev;
        let mut serial = cx.shared.serial;

        (&mut usb_dev, &mut serial).lock(|usb_dev, serial| {
            if !usb_dev.poll(&mut [serial]) {
                return;
            }

            let mut buf = [0u8; 32];

            match serial.read(&mut buf) {
                Ok(count) if count > 0 => {
                    decode::spawn(buf, count).unwrap();
                }
                _ => {}
            }
        });
    }

    #[task(binds = USB_LP_CAN_RX0, shared = [usb_dev, serial])]
    fn usb_rx0(cx: usb_rx0::Context) {
        let mut usb_dev = cx.shared.usb_dev;
        let mut serial = cx.shared.serial;

        (&mut usb_dev, &mut serial).lock(|usb_dev, serial| {
            if !usb_dev.poll(&mut [serial]) {
                return;
            }
            let mut buf = [0u8; 32];

            match serial.read(&mut buf) {
                Ok(count) if count > 0 => {
                    decode::spawn(buf, count).unwrap();
                }
                _ => {}
            }
        });
    }

    #[task(local = [led, state], shared = [data])]
    fn blink(cx: blink::Context) {
        if *cx.local.state {
            cx.local.led.set_high();
            *cx.local.state = false;
        } else {
            cx.local.led.set_low();
            *cx.local.state = true;
        }

        let mut data = cx.shared.data;
        data.lock(|data| {
            if let Cmd::Status(r, _) = data {
                *r = if *r == Relay::Close {
                    Relay::Open
                } else {
                    Relay::Close
                };
            }
        });
        blink::spawn_after(Duration::<u64, 1, 1000>::from_ticks(1000)).unwrap();
        start_read::spawn().unwrap();
    }

    #[task(shared = [relay])]
    fn set_relay(cx: set_relay::Context, state: Relay) {
        let mut relay = cx.shared.relay;
        relay.lock(|relay| match state {
            Relay::Open => relay.set_low(),
            Relay::Close => relay.set_high(),
        });
    }

    #[task(shared = [aht20rtic])]
    fn calibrate(cx: calibrate::Context) {
        let mut aht20rtic = cx.shared.aht20rtic;
        aht20rtic.lock(|aht20rtic| match aht20rtic.calibrated() {
            Ok(_) => rprintln!("calibrated"),
            Err(e) => rprintln!("NOT CALIBRATED: {:?}", e),
        });
        start_read::spawn_after(Duration::<u64, 1, 1000>::from_ticks(10)).unwrap();
    }

    #[task(shared = [aht20rtic])]
    fn wait_calibrate(cx: wait_calibrate::Context) {
        let mut aht20rtic = cx.shared.aht20rtic;
        aht20rtic.lock(|aht20rtic| {
            if aht20rtic.busy().unwrap() {
                wait_calibrate::spawn_after(Duration::<u64, 1, 1000>::from_ticks(10)).unwrap();
            } else {
                calibrate::spawn().unwrap();
            }
        });
    }

    #[task(shared = [aht20rtic])]
    fn start_read(cx: start_read::Context) {
        let mut aht20rtic = cx.shared.aht20rtic;
        aht20rtic.lock(|aht20rtic| aht20rtic.start_read().unwrap());
        wait_read::spawn_after(Duration::<u64, 1, 1000>::from_ticks(80)).unwrap();
    }

    #[task(shared = [aht20rtic])]
    fn wait_read(cx: wait_read::Context) {
        let mut aht20rtic = cx.shared.aht20rtic;
        aht20rtic.lock(|aht20rtic| {
            if aht20rtic.busy().unwrap() {
                wait_read::spawn_after(Duration::<u64, 1, 1000>::from_ticks(10)).unwrap();
            } else {
                end_read::spawn().unwrap();
            }
        });
    }

    #[task(shared = [aht20rtic, sensor])]
    fn end_read(cx: end_read::Context) {
        let sensor = cx.shared.sensor;
        let aht20rtic = cx.shared.aht20rtic;
        (sensor, aht20rtic).lock(|sensor, aht20rtic| {
            let msg = match aht20rtic.end_read() {
                Ok((h, t)) => SensorResult::Ok(SensorOk {
                    h: h.raw(),
                    t: t.raw(),
                }),
                Err(Error::Uncalibrated) => SensorResult::Err(SensorErr::Uncalibrated),
                Err(Error::Checksum) => SensorResult::Err(SensorErr::CheckSum),
                Err(Error::Bus(_)) => SensorResult::Err(SensorErr::Bus),
            };
            *sensor = msg;
        });
        start_read::spawn_after(Duration::<u64, 1, 1000>::from_ticks(5000)).unwrap();
    }

    #[task(shared = [relay, sensor])]
    fn send_status(cx: send_status::Context) {
        let sensor = cx.shared.sensor;
        let relay = cx.shared.relay;
        (sensor, relay).lock(|sensor, relay| {
            let cmd = Cmd::Status(
                if relay.is_set_high() {
                    Relay::Open
                } else {
                    Relay::Close
                },
                *sensor,
            );
            send::spawn(cmd).unwrap()
        });
    }

    #[task(shared = [serial])]
    fn send(cx: send::Context, cmd: Cmd) {
        rprintln!("send {:?}", cmd);
        let mut serial = cx.shared.serial;
        serial.lock(|serial| {
            serial.write(&HEADER).ok();
            let conf = bincode::config::standard();
            let mut buf = [0u8; 32];
            let size =
                encode_into_slice::<&Cmd, bincode::config::Configuration>(&cmd, &mut buf, conf)
                    .unwrap();
            serial.write(&[size.try_into().unwrap()]).ok();
            //rprintln!("encoded {} : {:?}", size, buf);
            serial.write(&buf[0..size]).ok();
        });
    }
}
