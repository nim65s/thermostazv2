check := "cargo check --color always"
clippy := "cargo clippy --color always"
embed := "cargo embed --release --package thermostazv2-fw --target thumbv7m-none-eabi"
test := "cargo test --color always"
clippy_w := "-W clippy::pedantic -W clippy::nursery -W clippy::unwrap_used -W clippy::expect_used"
clippy_a := "-A clippy::missing-errors-doc -A clippy::missing-panics-doc"
clippy_args := "-- " + clippy_w + " " + clippy_a
lib := "-p thermostazv2-lib"
drv := "-p thermostazv2-drv"
fw := "--target=thumbv7m-none-eabi -p thermostazv2-fw"

check-lib:
    {{check}} {{lib}}

check-drv:
    {{check}} {{drv}}

check-fw:
    {{check}} {{fw}}

clippy-lib:
    {{clippy}} {{lib}} {{clippy_args}}

clippy-drv:
    {{clippy}} {{drv}} {{clippy_args}}

clippy-fw:
    {{clippy}} {{fw}} {{clippy_args}}

test:
    {{test}} {{lib}}

embed:
    {{embed}}
