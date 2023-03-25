//! This shows some of the interrupts that can be generated by UART/Serial.
//! Use a proper serial terminal to connect to the board (espmonitor and
//! espflash won't work)

#![no_std]
#![no_main]

use core::{cell::RefCell, fmt::Write};

use critical_section::Mutex;
use esp_backtrace as _;
use hal::{
    clock::ClockControl,
    interrupt,
    peripherals::{self, Peripherals, UART0},
    prelude::*,
    riscv,
    timer::TimerGroup,
    uart::config::AtCmdConfig,
    Cpu, Rtc, Uart,
};
use nb::block;

static SERIAL: Mutex<RefCell<Option<Uart<UART0>>>> = Mutex::new(RefCell::new(None));

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::boot_defaults(system.clock_control).freeze();

    let mut rtc = Rtc::new(peripherals.RTC_CNTL);
    let mut serial0 = Uart::new(peripherals.UART0);
    let timer_group0 = TimerGroup::new(peripherals.TIMG0, &clocks);
    let mut timer0 = timer_group0.timer0;
    let mut wdt0 = timer_group0.wdt;
    let timer_group1 = TimerGroup::new(peripherals.TIMG1, &clocks);
    let mut wdt1 = timer_group1.wdt;

    // Disable watchdog timers
    rtc.swd.disable();
    rtc.rwdt.disable();
    wdt0.disable();
    wdt1.disable();

    serial0.set_at_cmd(AtCmdConfig::new(None, None, None, b'#', None));
    serial0.set_rx_fifo_full_threshold(30);
    serial0.listen_at_cmd();
    serial0.listen_rx_fifo_full();

    timer0.start(1u64.secs());

    critical_section::with(|cs| SERIAL.borrow_ref_mut(cs).replace(serial0));

    interrupt::enable(
        peripherals::Interrupt::UART0,
        interrupt::Priority::Priority1,
    )
    .unwrap();
    interrupt::set_kind(
        Cpu::ProCpu,
        interrupt::CpuInterrupt::Interrupt1, // Interrupt 1 handles priority one interrupts
        interrupt::InterruptKind::Edge,
    );

    unsafe {
        riscv::interrupt::enable();
    }

    loop {
        critical_section::with(|cs| {
            writeln!(SERIAL.borrow_ref_mut(cs).as_mut().unwrap(), "Hello World! Send a single `#` character or send at least 30 characters and see the interrupts trigger.").ok();
        });

        block!(timer0.wait()).unwrap();
    }
}

#[interrupt]
fn UART0() {
    critical_section::with(|cs| {
        let mut serial = SERIAL.borrow_ref_mut(cs);
        let serial = serial.as_mut().unwrap();

        let mut cnt = 0;
        while let nb::Result::Ok(_c) = serial.read() {
            cnt += 1;
        }
        writeln!(serial, "Read {cnt} bytes").ok();

        writeln!(
            serial,
            "Interrupt AT-CMD: {} RX-FIFO-FULL: {}",
            serial.at_cmd_interrupt_set(),
            serial.rx_fifo_full_interrupt_set(),
        )
        .ok();

        serial.reset_at_cmd_interrupt();
        serial.reset_rx_fifo_full_interrupt();
    });
}
