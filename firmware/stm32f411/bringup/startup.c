/* Minimal startup for STM32F411: vector table + reset handler. */
#include <stdint.h>

extern uint32_t _sidata, _sdata, _edata, _sbss, _ebss, _estack;

int main(void);

void Reset_Handler(void) {
    /* copy .data from flash to RAM */
    uint32_t *src = &_sidata, *dst = &_sdata;
    while (dst < &_edata) *dst++ = *src++;
    /* zero .bss */
    for (dst = &_sbss; dst < &_ebss;) *dst++ = 0;
    main();
    for (;;) {}
}

void Default_Handler(void) {
    for (;;) {}
}

/* Only the core system handlers are named; peripheral IRQs are unused. */
__attribute__((section(".isr_vector"), used))
void (*const g_vectors[16 + 86])(void) = {
    (void (*)(void)) & _estack, /* initial SP */
    Reset_Handler,              /* reset */
    Default_Handler,            /* NMI */
    Default_Handler,            /* HardFault */
    Default_Handler,            /* MemManage */
    Default_Handler,            /* BusFault */
    Default_Handler,            /* UsageFault */
    0, 0, 0, 0,                 /* reserved */
    Default_Handler,            /* SVCall */
    Default_Handler,            /* DebugMon */
    0,                          /* reserved */
    Default_Handler,            /* PendSV */
    Default_Handler,            /* SysTick */
    /* the rest (external IRQs) default to Default_Handler via the initialiser
     * below being zero -> but a 0 entry would fault if taken; we leave them 0
     * since no peripheral interrupts are enabled in this demo. */
};
