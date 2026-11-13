format binary as 'gba'

include '../lib/constants.inc'
include '../lib/macros.inc'

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

        add     r3, REG_TIM0CNT

        mov     r1, MEM_EWRAM

        mov     r4, r6
        adr     r4, .cnt_tmr

        mov     r5, MEM_GAMEPAK0
        adr     r5, .dma_cnt

        ldr     r5, [r5]

        b       t001

align 4
.wait_data:
        dw      0x00004014
align 4
.cnt_tmr:
        dw      0x00830000
align 4
.dma_cnt:
        dw      0x85010000

t001:
        ; loading a register from EWRAM should add one instruction
        ; to the prefetrcher for each load
        ; tests what happens when branching to nearby addresses
        mov     r7, MEM_IO

        str     r1, [r7, REG_DMA3DAD]
        mov     r1, MEM_IWRAM
        str     r1, [r7, REG_DMA3SAD]

        mov     r0, 0
        str     r0, [r3]
        ldr     r0, [r4]
        str     r0, [r3]
        str     r5, [r7, REG_DMA3CNT]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3]

        mov     r1, MEM_GAMEPAK0
        adr     r1, .tmr_rd

        ldr     r1, [r1]

        cmp     r0,r1
        bne     f001a

        bl      eval


.tmr_rd:
        dw      0x008301C0

f001a:
        mov     r12, 1
        bl      eval

eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
