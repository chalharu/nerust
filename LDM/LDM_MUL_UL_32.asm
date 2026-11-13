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
        dw      0x00001111
        dw      0x00001111
        dw      0x11111111
        dw      0x11111111


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

        mov     r5, r6
        adr     r5, .data_r10
        ldr     r10, [r5]

        mov     r5, r6
        adr     r5, .data_r11
        ldr     r11, [r5]

        mov     r5, r6
        adr     r5, .data_r3
        ldr     r3, [r5]

        mov     r5, r6
        adr     r5, .data_r4
        ldr     r4, [r5]

        mov     r5, r6
        adr     r5, .data_r5
        ldr     r5, [r5]

        mov     r12, r6

        adr     r6, .data_r6
        ldr     r6, [r6]

        ldm     r1, {r8, r9, r10, r11}^
        umlal   r8, r9, r10, r11
        mov     r0, r0
        umlal   r3, r4, r5, r6

        cmp     r3, r8
        bne     f001b

        cmp     r4, r9
        bne     f001c

        mov     r12, 0
        bl      eval

.mode_data:
        dw      0xF0000011

.data_r8:
        dw      0x11110000

.data_r9:
        dw      0x11110000

.data_r10:
        dw      0x01010101

.data_r11:
        dw      0x01010101

.data_r3:
        dw      0x11111111

.data_r4:
        dw      0x11111111

.data_r5:
        dw      0x01010101

.data_r6:
        dw      0x01010101

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
