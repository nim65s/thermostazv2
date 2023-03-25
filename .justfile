port := "/dev/ttyUSB0"
check := "cargo check --color always"
clippy := "cargo clippy --color always"
embed := "cargo embed --release --package thermostazv2-stm32 --target thumbv7m-none-eabi"
flash := "cargo espflash --target=riscv32imc-unknown-none-elf --monitor --release --package thermostazv2-esp32 " + port
test := "cargo test --color always"
clippy_w := "-W clippy::pedantic -W clippy::nursery -W clippy::unwrap_used -W clippy::expect_used"
clippy_a := "-A clippy::missing-errors-doc -A clippy::missing-panics-doc"
clippy_args := "-- " + clippy_w + " " + clippy_a
lib := "-p thermostazv2-lib"
drv := "-p thermostazv2-drv"
stm32 := "--target=thumbv7m-none-eabi -p thermostazv2-stm32"
esp32 := "--target=riscv32imc-unknown-none-elf -p thermostazv2-esp32"

check-lib:
    {{check}} {{lib}}

check-drv:
    {{check}} {{drv}}

check-stm32:
    {{check}} {{stm32}}

check-esp32:
    {{check}} {{esp32}}

clippy-lib:
    {{clippy}} {{lib}} {{clippy_args}}

clippy-drv:
    {{clippy}} {{drv}} {{clippy_args}}

clippy-stm32:
    {{clippy}} {{stm32}} {{clippy_args}}

clippy-esp32:
    {{clippy}} {{esp32}} {{clippy_args}}

test:
    {{test}} {{lib}}

embed:
    {{embed}}

flash:
    {{flash}}

all: clippy-lib clippy-drv clippy-stm32 clippy-esp32 test
