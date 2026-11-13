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

        mov     r4, r6
        adr     r4, .cnt_tmr

        mov     r0, 0
        adr     r0, t001 + 1
        bx      r0

align 4
.wait_data:
        dw      0x00004014

.cnt_tmr:
        dw      0x00800000

code16
align 2

t001:
        ; tests what happens with noregs selected in push
        ldr     r0, [r4]
        str     r0, [r3]

        mov     r0, r0

        dh      0xB400

        mov     r0, r0
        ldr     r0, [r3]
        mov     r7, 0xFF
        and     r0, r7
        cmp     r0, 0x6
        bne     f001a

        pop     {r0}
        and     r0, r7
        cmp     r0, 0x5C
        bne     f001b

        mov     r2, 0

        b       test_end

f001a:
        mov     r2, r0
        bl      test_end

f001b:
        mov     r2, r0
        bl      test_end

test_end:
        mov     r0, 0
        str     r0, [r3]
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
