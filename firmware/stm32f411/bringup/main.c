/*
 * open-tobii STM32F411 OLED bring-up demo (bare metal, no HAL).
 *
 * Proves the SSD1306 driver, the I2C wiring (PB6 SCL / PB7 SDA) and flashing:
 * it draws a dot that sweeps across the OLED — a stand-in for the gaze point
 * until the USB-host layer exists. The chip runs on its reset default clock
 * (HSI, 16 MHz), so no PLL setup is needed; APB1 = 16 MHz.
 */
#include "regs.h"
#include "ssd1306.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define I2C_TIMEOUT 100000u

static void delay(volatile uint32_t n) {
    while (n--) __asm__ volatile("nop");
}

static void gpio_i2c_init(void) {
    RCC_AHB1ENR |= RCC_AHB1ENR_GPIOBEN;
    RCC_APB1ENR |= RCC_APB1ENR_I2C1EN;

    /* PB6, PB7 -> alternate function (10), open-drain, pull-up, AF4 (I2C1). */
    GPIOB_MODER &= ~((3u << (6 * 2)) | (3u << (7 * 2)));
    GPIOB_MODER |= (2u << (6 * 2)) | (2u << (7 * 2));
    GPIOB_OTYPER |= (1u << 6) | (1u << 7);
    GPIOB_OSPEEDR |= (3u << (6 * 2)) | (3u << (7 * 2));
    GPIOB_PUPDR &= ~((3u << (6 * 2)) | (3u << (7 * 2)));
    GPIOB_PUPDR |= (1u << (6 * 2)) | (1u << (7 * 2));
    GPIOB_AFRL &= ~((0xFu << (6 * 4)) | (0xFu << (7 * 4)));
    GPIOB_AFRL |= (4u << (6 * 4)) | (4u << (7 * 4));

    /* I2C1 at 100 kHz, APB1 = 16 MHz. */
    I2C1_CR1 = I2C_CR1_SWRST;
    I2C1_CR1 = 0;
    I2C1_CR2 = 16;      /* FREQ = 16 MHz */
    I2C1_CCR = 80;      /* standard mode: 16M/(2*100k) */
    I2C1_TRISE = 17;    /* 16 + 1 */
    I2C1_CR1 = I2C_CR1_PE;
}

/* Blocking master-transmit of `len` bytes to a 7-bit address. */
static bool i2c_write(uint8_t addr, const uint8_t *data, size_t len) {
    uint32_t t;

    t = I2C_TIMEOUT;
    while ((I2C1_SR2 & I2C_SR2_BUSY) && --t) {}
    if (!t) return false;

    I2C1_CR1 |= I2C_CR1_START;
    t = I2C_TIMEOUT;
    while (!(I2C1_SR1 & I2C_SR1_SB) && --t) {}
    if (!t) return false;

    I2C1_DR = (uint32_t)(addr << 1); /* write */
    t = I2C_TIMEOUT;
    while (!(I2C1_SR1 & I2C_SR1_ADDR) && --t) {}
    if (!t) {
        I2C1_CR1 |= I2C_CR1_STOP;
        return false;
    }
    (void)I2C1_SR1;
    (void)I2C1_SR2; /* clear ADDR */

    for (size_t i = 0; i < len; i++) {
        t = I2C_TIMEOUT;
        while (!(I2C1_SR1 & I2C_SR1_TXE) && --t) {}
        if (!t) {
            I2C1_CR1 |= I2C_CR1_STOP;
            return false;
        }
        I2C1_DR = data[i];
    }

    t = I2C_TIMEOUT;
    while (!(I2C1_SR1 & I2C_SR1_BTF) && --t) {}
    I2C1_CR1 |= I2C_CR1_STOP;
    return true;
}

static void led_init(void) {
    RCC_AHB1ENR |= RCC_AHB1ENR_GPIOCEN;
    GPIOC_MODER &= ~(3u << (13 * 2));
    GPIOC_MODER |= (1u << (13 * 2)); /* output */
    GPIOC_BSRR = LED_OFF;
}
static void led(bool on) { GPIOC_BSRR = on ? LED_ON : LED_OFF; }
static void blink(int times, uint32_t on_ms, uint32_t off_ms) {
    for (int i = 0; i < times; i++) {
        led(true);
        delay(on_ms * 1600); /* ~1600 nop/ms at 16 MHz, rough */
        led(false);
        delay(off_ms * 1600);
    }
}

int main(void) {
    led_init();
    blink(3, 120, 120); /* firmware-alive proof */

    gpio_i2c_init();

    /* Probe the OLED: does it ACK on the bus? (display-off command) */
    static const uint8_t probe[] = {0x00, 0xAE};
    if (!i2c_write(OPEN_TOBII_OLED_I2C_ADDR, probe, sizeof probe)) {
        /* No ACK -> I2C/wiring/address problem. Fast blink forever. */
        for (;;) blink(1, 60, 60);
    }
    /* I2C OK: steady LED on while we drive the panel. If the screen stays dark
     * now, it is the panel init / VCC (SSD1309 charge pump), not the bus. */
    led(true);

    static struct ssd1306 oled;
    ssd1306_init(&oled, i2c_write, OPEN_TOBII_OLED_I2C_ADDR);

    /* Sweep a dot like a 1-D gaze; a visible proof the whole path works. */
    float x = 0.0f;
    float step = 0.02f;
    for (;;) {
        float y = 0.5f; /* middle row */
        ssd1306_show_gaze(&oled, x, y, true);
        x += step;
        if (x >= 1.0f) {
            x = 1.0f;
            step = -step;
        } else if (x <= 0.0f) {
            x = 0.0f;
            step = -step;
        }
        delay(300000);
    }
}
