/*
 * USB-host porting interface for the STM32F411 gaze reader.
 *
 * `main.c` is hardware-agnostic and calls only these functions; the STM32Cube
 * OTG_FS host + USB_HOST middleware implements them (milestone 2). An OLED
 * driver implements the display ones, built only when the display is enabled.
 */
#ifndef OPEN_TOBII_HOST_H
#define OPEN_TOBII_HOST_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* Find a supported Tobii tracker, set configuration 1, claim interface 0.
 * Returns true once the vendor interface (bulk 0x05/0x83) is ready. */
bool tobii_host_open(void);

/* Vendor control transfer. dir_in selects 0xC1 (IN) vs 0x41 (OUT).
 * Returns the byte count, or < 0 on error. */
int tobii_host_control(bool dir_in, uint8_t request, uint16_t value, uint8_t *buf, uint16_t len);

/* Bulk OUT to EP 0x05 (used by the init replay). Returns bytes sent or < 0. */
int tobii_host_bulk_out(const uint8_t *data, size_t len);

/* Bulk IN from EP 0x83. Returns bytes read (one frame), 0 on timeout, < 0 on
 * error. `timeout_ms` bounds the wait. */
int tobii_host_bulk_in(uint8_t *buf, size_t cap, uint32_t timeout_ms);

/* Debug console (UART or USB-CDC). */
void tobii_log(const char *msg);

#if OPEN_TOBII_ENABLE_DISPLAY
/* SSD1306 OLED, built only when the display option is on. */
void tobii_display_init(void);
/* Draw the gaze dot at normalised (x, y) in [0,1]; clears the previous one. */
void tobii_display_dot(float x, float y, bool valid);
#endif

#endif /* OPEN_TOBII_HOST_H */
