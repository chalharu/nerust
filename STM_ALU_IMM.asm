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
        str     r0, [r3, REG_IE]
        str     r0, [r3, REG_IME]

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]

        ; put data pointer in reg 1
        mov     r1, r6
        adr     r1, .user_data

        mov     r2, MEM_IWRAM
        mov     r0, 1
        str     r0, [r2]
        add     r2, 4
        mov     r0, 0
        str     r0, [r2]

        mov     r0, 0
        b       t001

align 4
.wait_data:
        dw      0x00000014

.user_data:
        dw      0x00000001

t001:
        ; tests STM^, no glitch occurs
        ; currently in system mode, change to FIRQ
        mov     r5, r6
        adr     r5, .mode_data
        ldr     r2, [r5]
        msr     cpsr, r2
        mrs     r2, cpsr
        and     r2, 0xFF
        cmp     r2, 0x11
        bne     f001a

        mov     r2, MEM_IWRAM
        ldm     r2, {r8}^

        mov     r0, r0
        mov     r5, 0
        mov     r8, 0
        mov     r3, r2
        add     r3, 4

        stm     r2, {r8}^
        add     r5, r8

        cmp     r5, 0
        bne     f001b

        mov     r5, 2
        stm     r2, {r8}^
        add     r8, r5

        stm     r2, {r8}^
        mov     r0, r0

        ldr     r0, [r2]
        cmp     r0, 1
        bne     f001c

        cmp     r8, 2
        bne     f001d

        mov     r8, 0

        stm     r2, {r8}^
        str     r8, [r3]

        ldr     r1, [r3]
        cmp     r1, 0
        bne     f001d

        mov     r12, 0
        bl      eval

.mode_data:
        dw      0xF0000011

f001a:
        mov     r12, 1
        bl      eval

f001b:
        mov     r12, 2
        bl      eval

f001c:
        mov     r12, 3
        bl      eval

f001d:
        mov     r12, 4
        bl      eval

f001e:
        mov     r12, 4
        bl      eval

eval:
        mov     r0, r12
        mov     r1, 0x1F
        msr     cpsr, r1

        m_vsync
        m_test_eval r0

idle:
        b       idle

include '../lib/text.asm'

main_end:
