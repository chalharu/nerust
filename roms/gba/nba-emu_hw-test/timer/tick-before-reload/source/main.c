#include <gba_base.h>
#include <gba_console.h>
#include <gba_interrupt.h>
#include <gba_timers.h>
#include <stdio.h>

#include "test.h"

// After enabling a timer, it takes one cycle to load the reload value into the counter.
// Alyosha discovered that the timer can tick in the cycle before loading the counter and signal an overflow.
IWRAM_CODE void test_tick_before_reload() {  
  // Reset TM0, TM1 and IF
  REG_TM0CNT = 0;
  REG_TM1CNT = 0;
  REG_IF = 0xFFFF;

  // Set TM0 counter = 0xFFFF
  REG_TM0CNT = TIMER_START << 16 | 0xFFFF;
  asm("nop;");
  REG_TM0CNT = 0;

  // Setup TM1 to count-up
  REG_TM1CNT_H = TIMER_START | TIMER_COUNT;

  // Setup TM0 to count from 0 on every clock cycle and generate an IRQ on overflow.
  REG_TM0CNT = (TIMER_START | TIMER_IRQ) << 16;
  asm("nop;");
  REG_TM0CNT = 0;

  test_expect("TM1CNT_L", 1, REG_TM1CNT_L);
  test_expect("IF", IRQ_TIMER0, REG_IF);
}

IWRAM_CODE int main(void) {
  consoleDemoInit();
  
  test_tick_before_reload();
  test_print_metrics();

  while (1) {
  }
}
