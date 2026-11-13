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
        mov     r5, MEM_IO
        add     r5, 0x200
        add     r5, 0x2

        mov     r6, MEM_GAMEPAK0
        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        str     r0, [r3, REG_SNDCNT]
        str     r0, [r3, REG_SNDCNTX]
        str     r0, [r3, REG_IE]
        str     r0, [r3, REG_IME]

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]

        ; set up dma 0 for reading timer
        ; set up dma 1 for writing timer

        str     r0, [r3, REG_DMA0SAD]
        str     r0, [r3, REG_DMA0DAD]
        str     r0, [r3, REG_DMA0CNT]

        str     r0, [r3, REG_DMA1SAD]
        str     r0, [r3, REG_DMA1DAD]
        str     r0, [r3, REG_DMA1CNT]

        mov     r0, MEM_EWRAM
        str     r0, [r3, REG_DMA0DAD]

        mov     r0, r3
        add     r0, REG_TIM0CNT
        str     r0, [r3, REG_DMA0SAD]

        mov     r0, MEM_IO
        add     r0, REG_TIM0CNT
        str     r0, [r3, REG_DMA1DAD]

        mov     r0, MEM_IWRAM
        str     r0, [r3, REG_DMA1SAD]

        mov     r5, r6
        adr     r5, .cnt_tmr
        ldr     r2, [r5]
        str     r2, [r0]

        mov     r5, r6
        adr     r5, .tmr_dma
        ldr     r2, [r5]

        mov     r4, r3
        add     r3, REG_DMA0CNT
        add     r4, REG_DMA1CNT

        mov     r1, MEM_EWRAM

        mov     r6, MEM_IO
        add     r6, REG_TIM0CNT

        mov     r0, 0
        adr     r0, t001 + 1
        bx      r0

align 4
.wait_data:
        dw      0x00004014

.cnt_tmr:
        dw      0x00C0FFE0

.tmr_dma:
        dw      0x84000001

code16
align 2

t001:
        ; tests what happens when timer
        ; is read immediately after enabled
        mov     r7, 0xFF

        ldr     r0, [r6]
        and     r0, r7
        cmp     r0, 0x8B

        bne     f001a

        mov     r0, 0
        ldr     r5, [r1]
        ldr     r5, [r1]
        str     r2, [r4]
        str     r2, [r3]
        ldr     r0, [r1]

        and     r0, r7
        cmp     r0, 0x8B
        bne     f001b

        ldr     r0, [r6]
        and     r0, r7
        cmp     r0, 0xF3

        bne     f001c

        mov     r2, 0

        b       test_end

f001a:
        mov     r2, 1
        bl      test_end

f001b:
        mov     r2, 2
        bl      test_end

f001c:
        mov     r2, 3
        bl      test_end


test_end:
        mov     r0, 0
        adr     r0, eval
        bx      r0

code32
align 4
eval:
        mov     r12, r2
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
