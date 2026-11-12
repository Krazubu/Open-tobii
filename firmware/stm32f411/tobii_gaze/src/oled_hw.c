/* Register-level I2C1 master for the OLED. APB1 = 42 MHz (BSP: HSE 25 -> PLL
 * N=336 P=4 -> SYSCLK 84, PCLK1 = 84/2 = 42 MHz). */
#include "oled_hw.h"

#define REG(a) (*(volatile uint32_t *)(a))
#define RCC_AHB1ENR REG(0x40023830u)
#define RCC_APB1ENR REG(0x40023840u)
#define GPIOB_MODER REG(0x40020400u)
#define GPIOB_OTYPER REG(0x40020404u)
#define GPIOB_OSPEEDR REG(0x40020408u)
#define GPIOB_PUPDR REG(0x4002040Cu)
#define GPIOB_AFRL REG(0x40020420u)
#define I2C1_CR1 REG(0x40005400u)
#define I2C1_CR2 REG(0x40005404u)
#define I2C1_DR REG(0x40005410u)
#define I2C1_SR1 REG(0x40005414u)
#define I2C1_SR2 REG(0x40005418u)
#define I2C1_CCR REG(0x4000541Cu)
#define I2C1_TRISE REG(0x40005420u)

#define PE (1u << 0)
#define START (1u << 8)
#define STOP (1u << 9)
#define SWRST (1u << 15)
#define SB (1u << 0)
#define ADDR (1u << 1)
#define BTF (1u << 2)
#define TXE (1u << 7)
#define BUSY (1u << 1)
#define TMO 200000u

void oled_i2c_init(void) {
    RCC_AHB1ENR |= (1u << 1);  /* GPIOB */
    RCC_APB1ENR |= (1u << 21); /* I2C1 */

    GPIOB_MODER &= ~((3u << 12) | (3u << 14));
    GPIOB_MODER |= (2u << 12) | (2u << 14); /* PB6/PB7 AF */
    GPIOB_OTYPER |= (1u << 6) | (1u << 7);  /* open drain */
    GPIOB_OSPEEDR |= (3u << 12) | (3u << 14);
    GPIOB_PUPDR &= ~((3u << 12) | (3u << 14));
    GPIOB_PUPDR |= (1u << 12) | (1u << 14); /* pull-up */
    GPIOB_AFRL &= ~((0xFu << 24) | (0xFu << 28));
    GPIOB_AFRL |= (4u << 24) | (4u << 28); /* AF4 = I2C1 */

    I2C1_CR1 = SWRST;
    I2C1_CR1 = 0;
    I2C1_CR2 = 42;    /* PCLK1 = 42 MHz */
    I2C1_CCR = 210;   /* 100 kHz std: 42e6/(2*100e3) */
    I2C1_TRISE = 43;  /* 42 + 1 */
    I2C1_CR1 = PE;
}

bool oled_i2c_write(uint8_t addr, const uint8_t *data, size_t len) {
    uint32_t t = TMO;
    while ((I2C1_SR2 & BUSY) && --t) {}
    if (!t) return false;
    I2C1_CR1 |= START;
    t = TMO;
    while (!(I2C1_SR1 & SB) && --t) {}
    if (!t) return false;
    I2C1_DR = (uint32_t)(addr << 1);
    t = TMO;
    while (!(I2C1_SR1 & ADDR) && --t) {}
    if (!t) { I2C1_CR1 |= STOP; return false; }
    (void)I2C1_SR1;
    (void)I2C1_SR2;
    for (size_t i = 0; i < len; i++) {
        t = TMO;
        while (!(I2C1_SR1 & TXE) && --t) {}
        if (!t) { I2C1_CR1 |= STOP; return false; }
        I2C1_DR = data[i];
    }
    t = TMO;
    while (!(I2C1_SR1 & BTF) && --t) {}
    I2C1_CR1 |= STOP;
    return true;
}
