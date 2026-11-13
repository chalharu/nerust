format binary as 'gba'

include '../lib/constants.inc'
include '../lib/macros.inc'

header:
        include '../lib/header.asm'

main:
        ; Setup DISPCNT
        mov     r0, 1 shl 8
        mov     r1, MEM_IO
        str     r0, [r1, REG_DISPCNT]

        ; Setup BG0CNT
        mov     r0, 0x41 shl 2
        mov     r1, MEM_IO
        str     r0, [r1, REG_BG0CNT]

        ; Setup color 1
        m_half  r0, 0x560B
        mov     r1, MEM_PALETTE
        strh    r0, [r1]

        ; Setup green color
        m_half  r0, 0x6290
        mov     r1, MEM_PALETTE
        strh    r0, [r1, 2]

        ; Generate tiles
        m_word  r0, 0x11551155
        mov     r1, MEM_VRAM
        add     r1, 0x4000
        mov     r2, 32

.loop_tile:
        subs    r2, 4
        str     r0, [r1, r2]
        bne     .loop_tile

        ; Generate map in alternating order
        m_word  r0, 0x00000400
        mov     r1, MEM_VRAM
        add     r1, 0x800
        mov     r2, 0x100

.loop_map:
        str     r0, [r1]
        add     r1, 4
        str     r0, [r1]
        add     r1, 4
        str     r0, [r1]
        add     r1, 4
        str     r0, [r1]
        add     r1, 4
        str     r0, [r1]
        add     r1, 4
        str     r0, [r1]
        add     r1, 4
        str     r0, [r1]
        add     r1, 4
        str     r0, [r1]
        add     r1, 4
        strh    r0, [r1]
        add     r1, 2
        strh    r0, [r1]
        add     r1, 2
        strh    r0, [r1]
        add     r1, 2
        strh    r0, [r1]
        add     r1, 10
        strh    r0, [r1]
        add     r1, 2
        strh    r0, [r1]
        add     r1, 2
        strh    r0, [r1]
        add     r1, 2
        strh    r0, [r1]
        add     r1, 10
        subs    r2, 4
        bne     .loop_map


        ;wait for vblank
        mov     r7, 0x1
        mov     r5, MEM_IO
        add     r5, 4
        mov     r6, MEM_IO
        add     r6, 16
        mov     r4, MEM_IO
        add     r4, 6

idle:
vb12:
        ldrh    r0, [r5]
        and     r0, r7
        cmp     r0, 1
        bne     vb12

        ; set x offset to 0
        mov    r0, 0
        str    r0, [r6]

        ; wait for scanline 45
        mov     r1, 45

ly1:
        ldrb    r0, [r4]
        cmp     r0, r1
        bne     ly1


        ; wait a few cycles and set x-offset to 0xFF
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0xFF
        str    r0, [r6]

        ; wait for scanline 90
        mov     r1, 90

ly2:
        ldrb    r0, [r4]
        cmp     r0, r1
        bne     ly2


        ; wait a few cycles and set x-offset to 0x77
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0
        mov    r0, 0x7A
        str    r0, [r6]

        b       idle