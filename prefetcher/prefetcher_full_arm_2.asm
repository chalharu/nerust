format binary as 'gba'

include '../lib/constants.inc'
include '../lib/macros.inc'

macro m_exit test {
        mov     r12, test
        b       eval
}

header:
        include '../lib/header.asm'

main:
        m_test_init
        ; Reset test register
        mov     r12, 0

        ; turn off sound
        mov     r3, MEM_IO
        mov     r6, MEM_GAMEPAK0
        mov     r0, 0
        str     r0, [r3, REG_SNDCNT]
        str     r0, [r3, REG_SNDCNTX]

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]

        b       t001

.wait_data:
        dw      0x00004014

t001:
        ; loading a register from EWRAM should add one instruction
        ; to the prefetrcher for each load
        ; fill the prefetcher then read from rom
        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]

        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]

        mov     r1, MEM_EWRAM
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        adr     r4, .cnt_tmr
        ldr     r4, [r4]

        mov     r0, r0
        mov     r0, r0
        mov     r0, r0

        ldr     r0, [r3, REG_TIM0CNT]
        and     r0, 0xFF

        cmp     r0, 0x6E
        bne     f001a

        b       eval

.cnt_tmr:
        dw      0x00800000

f001a:
        mov     r12, r0
        b       eval

eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
