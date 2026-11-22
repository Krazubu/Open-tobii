/* Build-time configuration for the open-tobii STM32F411 gaze reader. */
#ifndef OPEN_TOBII_CONFIG_H
#define OPEN_TOBII_CONFIG_H

/*
 * Optional OLED gaze dot. OFF by default so the firmware in the repository
 * builds without the display and its driver. Set to 1 to draw the gaze point
 * on an SSD1306 panel (the 0.42" 72x40 or any SSD1306).
 */
#ifndef OPEN_TOBII_ENABLE_DISPLAY
#define OPEN_TOBII_ENABLE_DISPLAY 0
#endif

/*
 * OLED panel geometry (used only when the display is enabled). Default: a 2.42"
 * 128x64 panel (SSD1306/SSD1309 — compatible command set), full width, no
 * column offset. For the 0.42" 72x40 panel use 72 / 40 / offset 28.
 */
#define OPEN_TOBII_OLED_WIDTH 128
#define OPEN_TOBII_OLED_HEIGHT 64
#define OPEN_TOBII_OLED_COL_OFFSET 0
/* 7-bit I2C address (0x3C -> 0x78 on the wire; 0x3D -> 0x7A). */
#define OPEN_TOBII_OLED_I2C_ADDR 0x3C

/*
 * I2C1 pins for the OLED (STM32F411 AF4). The 01Space 0.42" board wires its
 * panel to GPIO5/6 on an ESP32-C3; on the F411 any I2C1-capable pins work —
 * defaults below, override to match your wiring.
 */
#define OPEN_TOBII_OLED_SCL_PIN 6 /* PB6 */
#define OPEN_TOBII_OLED_SDA_PIN 7 /* PB7 */

/* Gaze console: print decoded gaze over the debug UART even without a display. */
#ifndef OPEN_TOBII_ENABLE_GAZE_LOG
#define OPEN_TOBII_ENABLE_GAZE_LOG 1
#endif

#endif /* OPEN_TOBII_CONFIG_H */
