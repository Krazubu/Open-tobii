# Bare-metal OLED bring-up demo for the STM32F411 (no HAL).
# `make` builds; `make flash` writes it over DFU with STM32CubeProgrammer.

CROSS   ?= arm-none-eabi-
CC      := $(CROSS)gcc
OBJCOPY := $(CROSS)objcopy
SIZE    := $(CROSS)size

CLI ?= /Applications/STMicroelectronics/STM32Cube/STM32CubeProgrammer/STM32CubeProgrammer.app/Contents/MacOs/bin/STM32_Programmer_CLI

CFLAGS  := -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -Os -g \
           -Wall -Wextra -ffreestanding -fno-common -fdata-sections -ffunction-sections \
           -I../include
LDFLAGS := -T stm32f411.ld -nostartfiles -Wl,--gc-sections -Wl,-Map=oled_demo.map

SRC := startup.c main.c ../src/ssd1306.c
TARGET := oled_demo

all: $(TARGET).bin

$(TARGET).elf: $(SRC) stm32f411.ld
	$(CC) $(CFLAGS) $(LDFLAGS) -o $@ $(SRC)
	$(SIZE) $@

$(TARGET).bin: $(TARGET).elf
	$(OBJCOPY) -O binary $< $@

# Board in DFU mode (hold BOOT0 + reset). port=usb1 per STM32_Programmer_CLI --list.
flash: $(TARGET).bin
	"$(CLI)" -c port=usb1 -w $(TARGET).bin 0x08000000 -v -g 0x08000000

clean:
	rm -f $(TARGET).elf $(TARGET).bin $(TARGET).map

.PHONY: all flash clean
