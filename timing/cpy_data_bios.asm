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

        mov     r10, MEM_EWRAM

        ; destination IWRAM
        mov     r1, MEM_IWRAM

        ; source is ROM
        mov     r0, MEM_GAMEPAK0

        ; count is 0xF0
        mov     r2, 0xF0

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r4, [r4]
        str     r4, [r3, REG_WAITCNT]

        b       t001

.wait_data:
        dw      0x00004014

t001:
        ; enable DMA and read timer
        mov     r4, 0
        str     r4, [r3, REG_TIM1CNT]
        mov     r4, r6
        adr     r4, .tmr_data
        ldr     r4, [r4]
        str     r4, [r3, REG_TIM1CNT]
        ldr     r9, [r10]
        ldr     r9, [r10]

        swi     0x0B0000

        mov     r3, MEM_IO
        ldr     r0,[r3, REG_TIM1CNT]

        and     r0, 0xFF

        cmp     r0, 0xA5
        bne     f001a

        b       eval

.tmr_data:
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
