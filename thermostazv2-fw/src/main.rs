//! CDC-ACM serial port example using cortex-m-rtic.
//! Target board: Blue Pill
//! with rtt
#![no_main]
#![no_std]
#![allow(non_snake_case)]

use panic_rtt_target as _;

#[rtic::app(device = stm32f1xx_hal::pac, peripherals = true, dispatchers = [SPI1, SPI2, SPI3, ADC1_2, ADC3, CAN_RX1, CAN_SCE])]
mod app {
    use aht20::{Aht20NoDelay, Error};
    use cortex_m::asm::delay;
    use rtt_target::{rprintln, rtt_init_print};
    use stm32f1xx_hal::gpio::PinState;
    use stm32f1xx_hal::gpio::{Alternate, OpenDrain, Output, PushPull, PB6, PB7, PB8, PC13};
    use stm32f1xx_hal::i2c::{BlockingI2c, DutyCycle, Mode};
    use stm32f1xx_hal::pac::I2C1;
    use stm32f1xx_hal::prelude::*;
    use stm32f1xx_hal::usb::{Peripheral, UsbBus, UsbBusType};
    use stm32f1xx_hal::watchdog::IndependentWatchdog;
    use systick_monotonic::{fugit::Duration, Systick};
    use thermostazv2_lib::{Cmd, Relay, SensorErr, SensorOk, SensorResult, TVec};
    use usb_device::prelude::*;

    type I2c = BlockingI2c<I2C1, (PB6<Alternate<OpenDrain>>, PB7<Alternate<OpenDrain>>)>;

    #[shared]
    struct Shared {
        aht20: Aht20NoDelay<I2c>,
        relay: PB8<Output<PushPull>>,
        sensor: SensorResult,
        serial: usbd_serial::SerialPort<'static, UsbBusType>,
    }

    #[local]
    struct Local {
        led: PC13<Output<PushPull>>,
        state: bool,
        iwdg: IndependentWatchdog,
        data: TVec,
        usb_dev: UsbDevice<'static, UsbBusType>,
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

        #[allow(clippy::unwrap_used)]
        let serial = usbd_serial::SerialPort::new(unsafe { USB_BUS.as_ref().unwrap() });

        #[allow(clippy::unwrap_used)]
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

        let Ok(aht20) = Aht20NoDelay::new(i2c) else {
            rprintln!("i2c write err");
            panic!();
        };

        let led = gpioc
            .pc13
            .into_push_pull_output_with_state(&mut gpioc.crh, PinState::Low);
        #[allow(clippy::unwrap_used)]
        blink::spawn_after(Duration::<u64, 1, 1000>::secs(1)).unwrap();
        #[allow(clippy::unwrap_used)]
        wait_calibrate::spawn_after(Duration::<u64, 1, 1000>::millis(20)).unwrap();
        rprintln!("init end");

        gpiob
            .pb9
            .into_push_pull_output_with_state(&mut gpiob.crh, PinState::Low);
        let relay = gpiob
            .pb8
            .into_push_pull_output_with_state(&mut gpiob.crh, PinState::Low);

        let sensor = SensorResult::Err(SensorErr::Uninitialized);

        let mut iwdg = IndependentWatchdog::new(cx.device.IWDG);
        iwdg.start(Duration::<u32, 1, 1000>::secs(3));

        (
            Shared {
                aht20,
                relay,
                sensor,
                serial,
            },
            Local {
                led,
                state: false,
                iwdg,
                data: TVec::new(),
                usb_dev,
            },
            init::Monotonics(mono),
        )
    }

    #[task(capacity = 3, local = [data, usb_dev], shared = [serial])]
    fn recv(cx: recv::Context) {
        let mut serial = cx.shared.serial;
        let usb_dev = cx.local.usb_dev;
        let data = cx.local.data;

        serial.lock(|serial| {
            if !usb_dev.poll(&mut [serial]) {
                return;
            }

            let mut buf = [0u8; 1];

            while serial.read(&mut buf).is_ok() {
                #[allow(clippy::unwrap_used)]
                data.push(buf[0]).unwrap();
                if buf[0] == 0 {
                    Cmd::from_vec(data).map_or_else(
                        |_| {
                            rprintln!("Couldn't decode {:?}", data);
                        },
                        |cmd| {
                            rprintln!("received {:?}", cmd);

                            #[allow(clippy::unwrap_used)]
                            match cmd {
                                Cmd::Get => send_status::spawn().unwrap(),
                                Cmd::Set(r) => set_relay::spawn(r).unwrap(),
                                Cmd::Ping => send::spawn(Cmd::Pong).unwrap(),
                                Cmd::Status(_, _) => rprintln!("wrong cmd received: {:?}", cmd),
                                Cmd::Pong => rprintln!("pong"),
                            }
                        },
                    );
                    data.clear();
                }
                if data.is_full() {
                    rprintln!("data is full: {:?}", data);
                    data.clear();
                }
            }
        });
    }

    #[task(binds = USB_HP_CAN_TX)]
    fn usb_tx(_cx: usb_tx::Context) {
        #[allow(clippy::unwrap_used)]
        recv::spawn().unwrap();
    }

    #[task(binds = USB_LP_CAN_RX0)]
    fn usb_rx0(_cx: usb_rx0::Context) {
        #[allow(clippy::unwrap_used)]
        recv::spawn().unwrap();
    }

    #[task(local = [led, state, iwdg])]
    fn blink(cx: blink::Context) {
        cx.local.iwdg.feed();
        if *cx.local.state {
            cx.local.led.set_high();
            *cx.local.state = false;
        } else {
            cx.local.led.set_low();
            *cx.local.state = true;
        }

        #[allow(clippy::unwrap_used)]
        blink::spawn_after(Duration::<u64, 1, 1000>::secs(1)).unwrap();
        //start_read::spawn().unwrap();
    }

    #[task(shared = [relay])]
    fn set_relay(cx: set_relay::Context, state: Relay) {
        let mut relay = cx.shared.relay;
        relay.lock(|relay| match state {
            Relay::Cold => relay.set_low(),
            Relay::Hot => relay.set_high(),
        });
    }

    #[task(shared = [aht20])]
    fn calibrate(cx: calibrate::Context) {
        let mut aht20 = cx.shared.aht20;
        aht20.lock(|aht20| match aht20.calibrated() {
            Ok(_) => rprintln!("calibrated"),
            Err(e) => rprintln!("NOT CALIBRATED: {:?}", e),
        });
        #[allow(clippy::unwrap_used)]
        start_read::spawn_after(Duration::<u64, 1, 1000>::millis(10)).unwrap();
    }

    #[task(shared = [aht20])]
    fn wait_calibrate(cx: wait_calibrate::Context) {
        let mut aht20 = cx.shared.aht20;
        aht20.lock(|aht20| {
            let Ok(busy) = aht20.busy() else {
                rprintln!("i2c write_read error");
                panic!();
            };
            if busy {
                #[allow(clippy::unwrap_used)]
                wait_calibrate::spawn_after(Duration::<u64, 1, 1000>::millis(10)).unwrap();
            } else {
                #[allow(clippy::unwrap_used)]
                calibrate::spawn().unwrap();
            }
        });
    }

    #[task(shared = [aht20])]
    fn start_read(cx: start_read::Context) {
        let mut aht20 = cx.shared.aht20;
        #[allow(clippy::unwrap_used)]
        aht20.lock(|aht20| aht20.start_read().unwrap());
        #[allow(clippy::unwrap_used)]
        wait_read::spawn_after(Duration::<u64, 1, 1000>::millis(80)).unwrap();
    }

    #[task(shared = [aht20])]
    fn wait_read(cx: wait_read::Context) {
        let mut aht20 = cx.shared.aht20;
        aht20.lock(|aht20| {
            let Ok(busy) = aht20.busy() else {
                rprintln!("i2c write_read error");
                panic!();
            };
            if busy {
                #[allow(clippy::unwrap_used)]
                wait_read::spawn_after(Duration::<u64, 1, 1000>::millis(10)).unwrap();
            } else {
                #[allow(clippy::unwrap_used)]
                end_read::spawn().unwrap();
            }
        });
    }

    #[task(shared = [aht20, sensor])]
    fn end_read(cx: end_read::Context) {
        let sensor = cx.shared.sensor;
        let aht20 = cx.shared.aht20;
        (sensor, aht20).lock(|sensor, aht20| {
            let msg = match aht20.end_read() {
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
        #[allow(clippy::unwrap_used)]
        send_status::spawn().unwrap();
        #[allow(clippy::unwrap_used)]
        start_read::spawn_after(Duration::<u64, 1, 1000>::secs(5)).unwrap();
    }

    #[task(shared = [relay, sensor])]
    fn send_status(cx: send_status::Context) {
        let sensor = cx.shared.sensor;
        let relay = cx.shared.relay;
        (sensor, relay).lock(|sensor, relay| {
            let cmd = Cmd::Status(Relay::from(relay.is_set_high()), *sensor);
            #[allow(clippy::unwrap_used)]
            send::spawn(cmd).unwrap();
        });
    }

    #[task(shared = [serial])]
    fn send(cx: send::Context, cmd: Cmd) {
        rprintln!("send {:?}", cmd);
        let mut serial = cx.shared.serial;
        cmd.to_vec().map_or_else(
            |e| rprintln!("{:?} to_vec err: {}", cmd, e),
            |vec| {
                serial.lock(|serial| serial.write(vec.as_slice()).ok());
            },
        );
    }
}
