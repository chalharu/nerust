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
        str     r0, [r3, REG_SNDCNT]
        str     r0, [r3, REG_SNDCNTX]
        str     r0, [r3, REG_IE]
        str     r0, [r3, REG_IME]

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]

        add     r3, REG_TIM0CNT

        mov     r1, MEM_EWRAM

        mov     r4, r6
        adr     r4, .cnt_tmr

        mov     r0, 0
        adr     r0, t001 + 1
        bx      r0

align 4
.wait_data:
        dw      0x00004014

.cnt_tmr:
        dw      0x00C0FFE0


code16
align 4

t001:
        ; tests what happens if only BL 1 or 2 is used
        mov     r2, 0
        mov     r0, r0
        ; BL1 then mov 55 to reg 2
        dw      0x2255F000
        mov     r3, 0x55
        cmp     r2, r3
        bne     fail_a

        mov     r0, 0
        ;BL 2 followed by mov 0x55 to r0
        dw      0x2055F805
        add     r0, 1
        add     r0, 1
        add     r0, 1
        add     r0, 1
        add     r0, 1
        add     r0, 1
        add     r0, 1
        add     r0, 1
        add     r0, 1
        add     r0, 1


        mov     r1, 0x5F
        cmp     r0, r1

        bne     fail_b


        b       test_end

fail_a:
        mov     r12, r2

fail_b:
        mov     r12, r0

test_end:
        mov     r0, 0
        str     r0, [r3]
        adr     r0, eval
        bx      r0

code32
align 4
eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
