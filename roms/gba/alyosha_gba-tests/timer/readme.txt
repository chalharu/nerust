Tests state of the clock divider after power on. Uses div 64 on timer 0.

Note this test likely won't work with flash carts like ex ezflash as it dependent on exact power on timing.

Test 2 tests enabling the timer when the current timer value is 0xFFFF. In this case an interrupt can be triggered
even though the timer should be reloaded. Presumably it ticks one cycle before resetting.
