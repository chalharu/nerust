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

        ; put sonme dat in OAM
        ; note byte sized writes noot allowed
        mov     r4, r6
        adr     r4, .oam_data
        ldr     r0, [r4]

        mov     r1, MEM_OAM
        str     r0, [r1]

        mov     r4, r6
        adr     r4, .oam_data_2
        ldr     r0, [r4]

        add     r1, 4
        str     r0, [r1]

        mov     r0, 0


        ; DMA 1 with immediate timing fixed addresses

        str     r0, [r3, REG_DMA1SAD]
        str     r0, [r3, REG_DMA1DAD]
        str     r0, [r3, REG_DMA1CNT]

        mov     r0, MEM_OAM
        str     r0, [r3, REG_DMA1DAD]

        mov     r5, r6
        adr     r5, .cnt_dma_src
        ldr     r0, [r5]
        str     r0, [r3, REG_DMA1SAD]


        mov     r5, r6
        adr     r5, .cnt_dma_1
        ldr     r2, [r5]
        str     r2, [r3+REG_DMA1CNT]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0

        ; now one DMA has run, setting up IWRM bus

        ; now set up DMA to read cpu open bus


        mov     r5, r6
        adr     r5, .cnt_dma_src_2
        ldr     r0, [r5]
        str     r0, [r3, REG_DMA1SAD]



        mov     r5, MEM_EWRAM
        str     r5, [r3, REG_DMA1DAD]
        mov     r5, MEM_OAM
        add     r5, 4
        mov     r4, MEM_EWRAM

        add     r3, REG_DMA1CNT


        mov     r0, 0
        adr     r0, t001 + 1
        bx      r0

align 4
.oam_data:
        dw      0x04030201

.oam_data_2:
        dw      0x08070605

.wait_data:
        dw      0x00004014

.cnt_dma_1:
        dw      0x85000001

.cnt_dma_src:
        dw      0x08000004

.cnt_dma_src_2:
        dw      0x01015000

code16
align 2

t001:
        ; tests DMA impact on OAM bus

        str     r2, [r3]
        ldrh    r0, [r5]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r7, 0xFF
        mov     r6, 16
        ldr     r2, [r4]
        lsr     r2, r6
        and     r2, r7
        cmp     r2, 0xAE

        bne     f001a

        mov     r2, 0
        b       test_end

f001a:
        ;mov     r2, 1
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
