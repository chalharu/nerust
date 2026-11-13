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

        mov     r0, 0
        adr     r0, t001 + 1
        bx      r0

align 4
.wait_data:
        dw      0x00000014

code16
align 2

t001:
        ; tests invalid encoding of blx
        mov     r1, 8

        adr     r0, test_bx
        ;bx r0 with high bit set
        dh      0x4780

        sub     r1, 1
        sub     r1, 1
        sub     r1, 1
        sub     r1, 1

        bx      r0

code32
align 4
test_bx:
        sub     r1, 1
        sub     r1, 1
        sub     r1, 1
        sub     r1, 1

        cmp     r1, 0x4

        bne     f001a

        b       eval

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
