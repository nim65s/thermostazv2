//! CDC-ACM serial port example using cortex-m-rtic.
//! Target board: Blue Pill
//! with bincode & rtt
#![no_main]
#![no_std]
#![allow(non_snake_case)]

//use panic_probe as _;
use panic_rtt_target as _;

#[rtic::app(device = stm32f1xx_hal::pac, peripherals = true, dispatchers = [SPI1])]
mod app {
    use cortex_m::asm::delay;
    use rtt_target::{rprintln, rtt_init_print};
    use stm32f1xx_hal::gpio::PinState;
    use stm32f1xx_hal::gpio::{gpioc::PC13, Output, PushPull};
    use stm32f1xx_hal::prelude::*;
    use stm32f1xx_hal::usb::{Peripheral, UsbBus, UsbBusType};
    use systick_monotonic::{fugit::Duration, Systick};
    use usb_device::prelude::*;
    use sheusrb_lib::*;

    #[shared]
    struct Shared {
        usb_dev: UsbDevice<'static, UsbBusType>,
        serial: usbd_serial::SerialPort<'static, UsbBusType>,
        data: C,
    }

    #[local]
    struct Local {
        led: PC13<Output<PushPull>>,
        state: bool,
    }

    #[monotonic(binds = SysTick, default = true)]
    type MonoTimer = Systick<1000>;

    #[init]
    fn init(cx: init::Context) -> (Shared, Local, init::Monotonics) {
        rtt_init_print!();
        rprintln!("hey");
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

        rprintln!("hello");

        let mut gpioa = cx.device.GPIOA.split();
        let mut gpioc = cx.device.GPIOC.split();

        // BluePill board has a pull-up resistor on the D+ line.
        // Pull the D+ pin down to send a RESET condition to the USB bus.
        // This forced reset is needed only for development, without it host
        // will not reset your device when you upload new firmware.
        let mut usb_dp = gpioa.pa12.into_push_pull_output(&mut gpioa.crh);
        usb_dp.set_low();
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
            UsbVidPid(0x6565, 0x0002),
        )
        .manufacturer("Nim")
        .product("thermostazv2")
        .serial_number("0001")
        .device_class(usbd_serial::USB_CLASS_CDC)
        .build();

        let data = C::B(B {
            goal: A {
                stop: true,
                pose: 42,
            },
            meas: A {
                stop: false,
                pose: 255,
            },
        });
        let led = gpioc
            .pc13
            .into_push_pull_output_with_state(&mut gpioc.crh, PinState::Low);
        blink::spawn_after(Duration::<u64, 1, 1000>::from_ticks(1000)).unwrap();

        (
            Shared {
                usb_dev,
                serial,
                data,
            },
            Local { led, state: false },
            init::Monotonics(mono),
        )
    }

    #[task(capacity = 3, shared = [data])]
    fn decode(cx: decode::Context, buf: [u8; 32], count: usize) {
        let conf = bincode::config::standard();
        let (decoded, size): (C, usize) = bincode::decode_from_slice(&buf, conf).unwrap();
        rprintln!("decode {} / {}: {:?}", size, count, decoded);
        let mut data = cx.shared.data;
        data.lock(|data| *data = decoded);
    }

    #[task(capacity = 3, shared = [data])]
    fn encode(cx: encode::Context) {
        let mut data = cx.shared.data;
        data.lock(|data| {
            let conf = bincode::config::standard();
            let mut buf = [0u8; 32];
            let size = bincode::encode_into_slice::<&C, bincode::config::Configuration>(
                data, &mut buf, conf,
            )
            .unwrap();
            rprintln!("encoded {} : {:?}", size, buf);
        });
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
        rprintln!("blink");
        if *cx.local.state {
            cx.local.led.set_high();
            *cx.local.state = false;
        } else {
            cx.local.led.set_low();
            *cx.local.state = true;
        }

        let mut data = cx.shared.data;
        data.lock(|data| match data {
            C::A(a) => a.stop ^= true,
            C::B(b) => b.meas.stop ^= true,
        });
        //encode::spawn().unwrap();
        blink::spawn_after(Duration::<u64, 1, 1000>::from_ticks(1000)).unwrap();
    }
}
