# open-tobii STM32F411 USB-host gaze reader, built with TinyUSB's make glue.
#
#   ../tools/setup_tinyusb.sh            # once: clone TinyUSB + deps into ../usb/
#   make BOARD=stm32f411blackpill        # display off (repository default)
#   make BOARD=stm32f411blackpill OLED=1   # with the OLED gaze dot
#
# TinyUSB lives outside git (../usb/tinyusb by default); override with
# TINYUSB=/path/to/tinyusb.

TINYUSB ?= $(abspath $(CURDIR)/../usb/tinyusb)
ifeq (,$(wildcard $(TINYUSB)/hw/bsp/family_support.mk))
$(error TinyUSB not found at $(TINYUSB) - run ../tools/setup_tinyusb.sh or set TINYUSB=)
endif

include $(TINYUSB)/hw/bsp/family_support.mk

# The portable driver (OLED + Tobii protocol) lives in the firmware tree.
FW := $(abspath $(CURDIR)/..)

OLED ?= 0
CFLAGS += -DOPEN_TOBII_ENABLE_DISPLAY=$(OLED)

INC += \
	$(CURDIR)/src \
	$(FW)/include \

SRC_C += \
	$(CURDIR)/src/main.c \
	$(CURDIR)/src/oled_hw.c \
	$(FW)/src/ssd1306.c \
	$(FW)/src/tobii_proto.c \

include $(TINYUSB)/hw/bsp/family_rules.mk
