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
        dw      0x00000001


t001:
        ; tests LDM^ with mul ul and sl
        ; currently in system mode, change to FIRQ
        mov     r5, r6
        adr     r5, .mode_data
        ldr     r2, [r5]
        msr     cpsr, r2
        mrs     r2, cpsr

        mov     r8, r6
        adr     r8, .bx_addr
        mov     r2, 1

        ldm     r1, {r8}^
        bx      r8

        nop
        nop
.bx_addr:
code16
        ; we should be in thumb mode now
        ; becaue LDM glitch or'd in a 1 to register 8
        add     r1, 1
        mov     r2, 0
        adr     r0, eval
        bx      r0

.mode_data:
        dw      0xF0000011

code32
align 4

eval:

        mov     r0, r2
        mov     r1, 0x1F
        msr     cpsr, r1

        m_vsync
        m_test_eval r0

idle:
        b       idle

include '../lib/text.asm'

main_end:
