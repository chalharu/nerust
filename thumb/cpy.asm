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


        mov     r1, 1
        mov     r2, 2
        mov     r3, 3
        mov     r4, 4
        mov     r5, 5
        mov     r6, 6
        mov     r7, 7

        mov     r0, 0
        adr     r0, t001 + 1
        bx      r0

align 4
.wait_data:
        dw      0x00000014

code16
align 2

t001:
        ; tests invalid encoding to cpy
        dh      0x460A

        cmp     r2, 0x1

        bne     f001a

        mov     r2, 0

        b       test_end

f001a:
        b       test_end

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
