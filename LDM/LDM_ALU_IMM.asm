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

        mov     r0, 0
        b       t001

align 4
.wait_data:
        dw      0x00000014

.user_data:
        dw      0x0000FFFF
        dw      0x0000FFFF


t001:
        ; tests LDM^ with mul ul and sl
        ; currently in system mode, change to FIRQ
        mov     r5, r6
        adr     r5, .mode_data
        ldr     r2, [r5]
        msr     cpsr, r2
        mrs     r2, cpsr
        and     r2, 0xFF
        cmp     r2, 0x11
        bne     f001a

        mov     r5, r6
        adr     r5, .data_r8
        ldr     r8, [r5]

        mov     r5, r6
        adr     r5, .data_r9
        ldr     r9, [r5]

        ldm     r1, {r8, r9, r10, r11}^
        add     r8, 1

        ldm     r1, {r8, r9, r10, r11}^
        add     r9, 0x8000

        cmp     r8, 0
        bne     f001b

        mov     r1, 0x8000
        sub     r1, 1
        cmp     r9, r1
        bne     f001c

        mov     r12, 0
        bl      eval

.mode_data:
        dw      0xF0000011

.data_r8:
        dw      0xFFFF0000

.data_r9:
        dw      0xFFFF0000

f001a:
        mov     r12, 1
        bl      eval

f001b:
        mov     r12, 2
        bl      eval

f001c:
        mov     r12, 3
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
