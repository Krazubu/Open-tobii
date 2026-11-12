/*
 * SSD1306 OLED driver (monochrome framebuffer), hardware-agnostic.
 *
 * Geometry comes from config.h (default 128x64; 72x40 for the 0.42" panel). I2C is done
 * through a caller-supplied write callback, so the same driver serves the
 * bare-metal bring-up demo and, later, the HAL-based firmware.
 */
#ifndef OPEN_TOBII_SSD1306_H
#define OPEN_TOBII_SSD1306_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "config.h"

#define SSD1306_W OPEN_TOBII_OLED_WIDTH
#define SSD1306_H OPEN_TOBII_OLED_HEIGHT
#define SSD1306_PAGES ((OPEN_TOBII_OLED_HEIGHT + 7) / 8)

/* Write `len` bytes to the panel's I2C address. Returns true on success. */
typedef bool (*ssd1306_i2c_write_fn)(uint8_t addr, const uint8_t *data, size_t len);

struct ssd1306 {
    ssd1306_i2c_write_fn i2c_write;
    uint8_t addr;
    uint8_t fb[SSD1306_W * SSD1306_PAGES];
};

/* Initialise the panel (sends the config command list). Returns false if any
 * I2C write fails (e.g. nothing on the bus). */
bool ssd1306_init(struct ssd1306 *d, ssd1306_i2c_write_fn i2c_write, uint8_t addr);

void ssd1306_clear(struct ssd1306 *d);
void ssd1306_set_pixel(struct ssd1306 *d, int x, int y, bool on);
/* A filled square of side `r*2+1` centred on (x,y) — a visible dot. */
void ssd1306_fill_dot(struct ssd1306 *d, int x, int y, int r);
/* Push the framebuffer to the panel. */
bool ssd1306_flush(struct ssd1306 *d);

/* Map a normalised gaze point (x,y in [0,1]) to pixels and draw it, clearing
 * the frame first. Flushes. */
bool ssd1306_show_gaze(struct ssd1306 *d, float x, float y, bool valid);

#endif /* OPEN_TOBII_SSD1306_H */
