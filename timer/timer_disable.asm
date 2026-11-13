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

        ; set up dma 1 for auto refill of FIFO A
        
        str     r0, [r3, REG_DMA1SAD]
        str     r0, [r3, REG_DMA1DAD]
        str     r0, [r3, REG_DMA1CNT]

        mov     r4, r6
        mov     r5, r6
        mov     r0, MEM_EWRAM
        adr     r4, .fifo_addr
        adr     r5, .fifo_dma
        ldr     r1, [r4]
        ldr     r2, [r5]

        str     r0, [r3, REG_DMA1SAD]
        str     r1, [r3, REG_DMA1DAD]
        str     r2, [r3, REG_DMA1CNT]

                ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]

        b       t001

.wait_data:
        dw      0x00004014

.fifo_addr:
        dw      0x040000A0

.fifo_dma:
        dw      0xB7400004

t001:
        ; test enabling timer when timer internally
        ; is at 0xFFFF
        mov     r10, MEM_EWRAM
        mov     r4, r6
        adr     r4, .clr_flags
        ldr     r0, [r4]
        str     r0, [r3, REG_IE]
        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r10]
        ldr     r0, [r10]
        mov     r0, r0
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r3, REG_TIM0CNT]
        and     r0, 0xFF
        cmp     r0, 0xFF
        bne     f001a
        mov     r4, r6
        adr     r4, .cnt_tmr_2
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]
        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        mov     r4, r6
        adr     r4, .flg_reg
        ldr     r3, [r4]
        ldrh    r0, [r3]
        cmp     r0, 8
        bne     f001b


        b       eval

.clr_flags:
        dw      0xFFFF0000

.cnt_tmr:
        dw      0x00C0FFEB

.cnt_tmr_2:
        dw      0x00C00000

.flg_reg:
        dw      0x04000202


f001a:
        mov     r12, r0
        b       eval

f001b:
        mov     r12, 2
        b       eval

eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
