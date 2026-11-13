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

.cnt_tmr:
        dw      0x00800000
        ; loading a register from EWRAM should add one instruction
        ; to the prefetrcher for each load
        ; tests what happens when branching to nearby addresses
        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        mov     r1, MEM_EWRAM
        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        b       branch_1
branch_1:
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3, REG_TIM0CNT]
        mov     r5, r6
        adr     r5, .tmr_read_a
        ldr     r1, [r5]
        cmp     r0, r1
        bne     f001a

.tmr_read_a:
        dw      0x0080003F

        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        mov     r1, MEM_EWRAM
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        b       branch_2
        mov     r0, r0
branch_2:
        mov     r0, r0
        ldr     r0, [r3, REG_TIM0CNT]
        mov     r5, r6
        adr     r5, .tmr_read_b
        ldr     r1, [r5]
        cmp     r0, r1
        bne     f001b

.tmr_read_b:
        dw      0x0080003B

        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        mov     r1, MEM_EWRAM
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        b       branch_3
        mov     r0, r0
        mov     r0, r0
branch_3:
        ldr     r0, [r3, REG_TIM0CNT]
        mov     r5, r6
        adr     r5, .tmr_read_c
        ldr     r1, [r5]
        cmp     r0, r1
        bne     f001c
        b       t002

.tmr_read_c:
        dw      0x0080002B


f001a:
        m_exit  1

f001b:
        m_exit  2

f001c:
        m_exit  3


t002:
        ; loading a register from EWRAM should add one instruction
        ; to the prefetrcher for each load
        ; tests what happens when trying to add more than 8 samples
        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        mov     r1, MEM_EWRAM
        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3, REG_TIM0CNT]
        mov     r4, r6
        adr     r4, .tmr_read_a
        ldr     r1, [r4]
        cmp     r0, r1
        bne     f002a

        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        mov     r1, MEM_EWRAM
        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3, REG_TIM0CNT]
        mov     r4, r6
        adr     r4, .tmr_read_b
        ldr     r1, [r4]
        cmp     r0, r1
        bne     f002b

        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        mov     r1, MEM_EWRAM
        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3, REG_TIM0CNT]
        mov     r4, r6
        adr     r4, .tmr_read_c
        ldr     r1, [r4]
        cmp     r0, r1
        bne     f002c

        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        mov     r1, MEM_EWRAM
        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3, REG_TIM0CNT]
        mov     r4, r6
        adr     r4, .tmr_read_d
        ldr     r1, [r4]
        cmp     r0, r1
        bne     f002d
        b       t003

.cnt_tmr:
        dw      0x00800000

.tmr_read_a:
        dw      0x00800021

.tmr_read_b:
        dw      0x0080002A

.tmr_read_c:
        dw      0x00800038

.tmr_read_d:
        dw      0x00800047

f002a:
        m_exit  11

f002b:
        m_exit  12

f002c:
        and     r0, 0xFF
        mov     r12, r0
        b       eval
        m_exit  13

f002d:
        m_exit  14

t003:
        ; loading a register from EWRAM should add one instruction
        ; to the prefetrcher for each load
        ; tests what happens when trying to add more than 8 samples
        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        mov     r1, MEM_EWRAM
        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r1]
        ldr     r0, [r3, REG_TIM0CNT]
        mov     r4, r6
        adr     r4, .tmr_read_a
        ldr     r1, [r4]
        cmp     r0, r1
        bne     f003a

        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        mov     r1, MEM_EWRAM
        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r1]
        ldr     r0, [r3, REG_TIM0CNT]
        mov     r4, r6
        adr     r4, .tmr_read_b
        ldr     r1, [r4]
        cmp     r0, r1
        bne     f003b

        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        mov     r1, MEM_EWRAM
        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r1]
        ldr     r0, [r3, REG_TIM0CNT]
        mov     r4, r6
        adr     r4, .tmr_read_c
        ldr     r1, [r4]
        cmp     r0, r1
        bne     f003c
        b       eval

.cnt_tmr:
        dw      0x00800000

.tmr_read_a:
        dw      0x00800039

.tmr_read_b:
        dw      0x0080003F

.tmr_read_c:
        dw      0x00800040

f003a:
        and     r0, 0xFF
        mov     r12, r0
        b       eval
        m_exit  11

f003b:
        and     r0, 0xFF
        mov     r12, r0
        b       eval
        m_exit  12

f003c:
        and     r0, 0xFF
        mov     r12, r0
        b       eval
        m_exit  13

eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
