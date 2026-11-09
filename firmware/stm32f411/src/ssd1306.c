/*
 * SSD1306 OLED driver. See ssd1306.h.
 * The 72x40 panel (0.42") sits offset inside the controller's 128x64 RAM;
 * OPEN_TOBII_OLED_COL_OFFSET places it.
 */
#include "ssd1306.h"

/* Send a command stream: control byte 0x00, then the commands. */
static bool cmd(struct ssd1306 *d, const uint8_t *c, size_t n) {
    uint8_t buf[24];
    if (n + 1 > sizeof buf) return false;
    buf[0] = 0x00; /* Co=0, D/C#=0: all following bytes are commands */
    for (size_t i = 0; i < n; i++) buf[1 + i] = c[i];
    return d->i2c_write(d->addr, buf, n + 1);
}

bool ssd1306_init(struct ssd1306 *d, ssd1306_i2c_write_fn i2c_write, uint8_t addr) {
    d->i2c_write = i2c_write;
    d->addr = addr;
    ssd1306_clear(d);

    static const uint8_t init[] = {
        0xAE,             /* display off */
        0xD5, 0x80,       /* clock divide / osc freq */
        0xA8, (uint8_t)(SSD1306_H - 1), /* multiplex ratio = height-1 (0x27 for 40) */
        0xD3, 0x00,       /* display offset 0 */
        0x40,             /* start line 0 */
        0x8D, 0x14,       /* charge pump on */
        0x20, 0x00,       /* memory addressing mode: horizontal */
        0xA1,             /* segment remap (column 127 -> SEG0) */
        0xC8,             /* COM scan direction remapped */
        0xDA, 0x12,       /* COM pins config */
        0x81, 0x7F,       /* contrast */
        0xD9, 0xF1,       /* pre-charge */
        0xDB, 0x40,       /* VCOMH deselect */
        0xA4,             /* resume to RAM content */
        0xA6,             /* normal (not inverted) */
        0xAF,             /* display on */
    };
    if (!cmd(d, init, sizeof init)) return false;
    return ssd1306_flush(d);
}

void ssd1306_clear(struct ssd1306 *d) {
    for (size_t i = 0; i < sizeof d->fb; i++) d->fb[i] = 0;
}

void ssd1306_set_pixel(struct ssd1306 *d, int x, int y, bool on) {
    if (x < 0 || x >= SSD1306_W || y < 0 || y >= SSD1306_H) return;
    uint8_t *cell = &d->fb[(size_t)(y / 8) * SSD1306_W + (size_t)x];
    uint8_t bit = (uint8_t)(1u << (y % 8));
    if (on) *cell |= bit;
    else *cell &= (uint8_t)~bit;
}

void ssd1306_fill_dot(struct ssd1306 *d, int x, int y, int r) {
    for (int dy = -r; dy <= r; dy++)
        for (int dx = -r; dx <= r; dx++)
            ssd1306_set_pixel(d, x + dx, y + dy, true);
}

bool ssd1306_flush(struct ssd1306 *d) {
    const uint8_t col0 = OPEN_TOBII_OLED_COL_OFFSET;
    uint8_t window[] = {
        0x21, col0, (uint8_t)(col0 + SSD1306_W - 1), /* column start/end */
        0x22, 0x00, (uint8_t)(SSD1306_PAGES - 1),    /* page start/end */
    };
    if (!cmd(d, window, sizeof window)) return false;

    /* data stream: control byte 0x40, then the whole framebuffer */
    uint8_t buf[1 + SSD1306_W * SSD1306_PAGES];
    buf[0] = 0x40;
    for (size_t i = 0; i < sizeof d->fb; i++) buf[1 + i] = d->fb[i];
    return d->i2c_write(d->addr, buf, sizeof buf);
}

bool ssd1306_show_gaze(struct ssd1306 *d, float x, float y, bool valid) {
    ssd1306_clear(d);
    if (valid) {
        int px = (int)(x * (float)(SSD1306_W - 1) + 0.5f);
        int py = (int)(y * (float)(SSD1306_H - 1) + 0.5f);
        ssd1306_fill_dot(d, px, py, 2);
    }
    return ssd1306_flush(d);
}
