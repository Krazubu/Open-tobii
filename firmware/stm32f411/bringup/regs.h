/* Minimal STM32F411 register map used by the bring-up demo (no CMSIS). */
#ifndef OPEN_TOBII_F411_REGS_H
#define OPEN_TOBII_F411_REGS_H

#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))

/* RCC */
#define RCC_BASE 0x40023800u
#define RCC_AHB1ENR REG(RCC_BASE + 0x30u)
#define RCC_APB1ENR REG(RCC_BASE + 0x40u)
#define RCC_AHB1ENR_GPIOBEN (1u << 1)
#define RCC_APB1ENR_I2C1EN (1u << 21)

/* GPIOB */
#define GPIOB_BASE 0x40020400u
#define GPIOB_MODER REG(GPIOB_BASE + 0x00u)
#define GPIOB_OTYPER REG(GPIOB_BASE + 0x04u)
#define GPIOB_OSPEEDR REG(GPIOB_BASE + 0x08u)
#define GPIOB_PUPDR REG(GPIOB_BASE + 0x0Cu)
#define GPIOB_AFRL REG(GPIOB_BASE + 0x20u)

/* I2C1 (APB1) */
#define I2C1_BASE 0x40005400u
#define I2C1_CR1 REG(I2C1_BASE + 0x00u)
#define I2C1_CR2 REG(I2C1_BASE + 0x04u)
#define I2C1_DR REG(I2C1_BASE + 0x10u)
#define I2C1_SR1 REG(I2C1_BASE + 0x14u)
#define I2C1_SR2 REG(I2C1_BASE + 0x18u)
#define I2C1_CCR REG(I2C1_BASE + 0x1Cu)
#define I2C1_TRISE REG(I2C1_BASE + 0x20u)

#define I2C_CR1_PE (1u << 0)
#define I2C_CR1_START (1u << 8)
#define I2C_CR1_STOP (1u << 9)
#define I2C_CR1_SWRST (1u << 15)
#define I2C_SR1_SB (1u << 0)
#define I2C_SR1_ADDR (1u << 1)
#define I2C_SR1_BTF (1u << 2)
#define I2C_SR1_TXE (1u << 7)
#define I2C_SR2_BUSY (1u << 1)

#endif /* OPEN_TOBII_F411_REGS_H */
