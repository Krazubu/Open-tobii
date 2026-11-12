#!/bin/sh
# Fetch TinyUSB and the STM32F4 dependencies the gaze firmware builds against,
# at the commits it was verified with on hardware, into firmware/stm32f411/usb/
# (ignored by git). Re-running it is safe: existing checkouts are left alone.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TUSB="$ROOT/usb/tinyusb"

fetch() { # url dir commit
    if [ -d "$2/.git" ]; then
        echo "keep  $2"
        return
    fi
    echo "fetch $1 @ $3"
    mkdir -p "$2"
    git -C "$2" init -q
    git -C "$2" remote add origin "$1"
    git -C "$2" fetch -q --depth 1 origin "$3"
    git -C "$2" checkout -q FETCH_HEAD
}

fetch https://github.com/hathach/tinyusb.git "$TUSB" \
    d543126c5ca356f6ddc52646ee5a2dee84c65c31
fetch https://github.com/ARM-software/CMSIS_5.git "$TUSB/lib/CMSIS_5" \
    2b7495b8535bdcb306dac29b9ded4cfb679d7e5c
fetch https://github.com/STMicroelectronics/cmsis_device_f4.git "$TUSB/hw/mcu/st/cmsis_device_f4" \
    3c77349ce04c8af401454cc51f85ea9a50e34fc1
fetch https://github.com/STMicroelectronics/stm32f4xx_hal_driver.git "$TUSB/hw/mcu/st/stm32f4xx_hal_driver" \
    b6f0ed3829f3829eb358a2e7417d80bba1a42db7

echo "TinyUSB ready in $TUSB"
