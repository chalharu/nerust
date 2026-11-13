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
        ; turn off sound
        mov     r3, MEM_IO
        mov     r6, MEM_GAMEPAK0
        mov     r0, 0
        str     r0, [r3, REG_SNDCNT]
        str     r0, [r3, REG_SNDCNTX]

        str     r0, [r3, REG_TIM0CNT]
        str     r0, [r3, REG_TIM1CNT]
        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]

        m_test_init
        ; Reset test register
        mov     r12, 0

        mov     r3, MEM_IO
        mov     r6, MEM_GAMEPAK0

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        ;ldr     r1, [r3, REG_TIM0CNT]
        str     r0, [r3, REG_WAITCNT]

        b       t001

.wait_data:
        dw      0x00004014

.cnt_tmr:
        dw      0x00800000

t001:
        ; test timing of DMA with prefetch enabled

        ldr     r0, [r3, REG_TIM0CNT]
        ;and     r1, 0xFF
        and     r0, 0xFF

        ;cmp     r1, 0xA8
        ;bne     f001a

        cmp     r0, 0xBD
        bne     f001b

        b       eval

f001a:
        m_exit  1


f001b:
        mov     r12, r0
        b       eval

        m_exit  1

eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
