/* I2C1 glue for the OLED on the STM32F411 (PB6 SCL / PB7 SDA), APB1 = 42 MHz
 * under the TinyUSB board clock. */
#ifndef OLED_HW_H
#define OLED_HW_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

void oled_i2c_init(void);
bool oled_i2c_write(uint8_t addr, const uint8_t *data, size_t len);

#endif
