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

        mov     r0, 0

        ; DMA 1 with immediate timing fixed addresses

        str     r0, [r3, REG_DMA1SAD]
        str     r0, [r3, REG_DMA1DAD]
        str     r0, [r3, REG_DMA1CNT]

        mov     r0, MEM_EWRAM
        str     r0, [r3, REG_DMA1DAD]

        mov     r5, r6
        adr     r5, .cnt_dma_src_2
        ldr     r0, [r5]
        str     r0, [r3, REG_DMA1SAD]


        mov     r5, r6
        adr     r5, .cnt_dma_1
        ldr     r2, [r5]
        add     r3, REG_DMA1CNT


        mov     r5, MEM_GAMEPAK0
        add     r5, 5
        mov     r4, MEM_EWRAM


        mov     r0, 0
        adr     r0, t001 + 1
        bx      r0

align 4

.wait_data:
        dw      0x00004014

.cnt_dma_1:
        dw      0x85000001

.cnt_dma_src_2:
        dw      0x04001000

code16
align 2

t001:
        ; tests signed half loads effect on bus
        mov     r0, 0
        str     r2, [r3]
        ldrsh   r0, [r5,r0]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r7, 0xFF
        mov     r6, 16
        ldr     r2, [r4]
        lsr     r2, r6
        and     r2, r7
        cmp     r2, 0x24

        bne     f001a

        mov     r2, 0
        b       test_end

f001a:
        mov     r2, 1
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
