;Lan Master NES game
;by Shiru (shiru@mail.ru) 05'11
;Compile with NESASM3
;The game and its source code are released into Public Domain

    .inesprg    2
    .ineschr    1
    .inesmir    1
    .inesmap    0

    .bank 0
    .org $8000

PPU_CTRL		equ $2000
PPU_MASK		equ $2001
PPU_STATUS		equ $2002
PPU_OAM_ADDR	equ $2003
PPU_SCROLL		equ $2005
PPU_ADDR		equ $2006
PPU_DATA		equ $2007
PPU_OAM_DMA		equ $4014
PPU_FRAMECNT	equ $4017
DMC_FREQ		equ $4010
CTRL_PORT1		equ $4016

TEMP			equ $00
PAD_BUF			equ $00	;3 bytes
PAL_LOW			equ $00
PAL_HIGH		equ $01
PAL_BR_LOW		equ $02
PAL_BR_HIGH		equ $03
PAL_CNT			equ $04

FRAME_CNT		equ $ff
FRAME_CNT2		equ $fe
RAND_SEED		equ $fd
NMI_CALL		equ $fa	;3 bytes, either RTI or JMP adr

PAD_STATE		equ $f9
PAD_STATEP		equ $f8
PAD_STATET		equ $f7
;f0..f6 for FamiTone vars

MENU_BASE		equ $ef
MENU_CUR		equ $ee
MENU_CNT		equ $ed
MENU_CURADRL	equ $ec
MENU_CURADRH	equ $eb
MENU_CODECUR	equ $ea
MENU_ANIM		equ $e9
MENU_SET		equ $e8

GAME_BASE		equ $e7
GAME_CUR_SX		equ $e6
GAME_CUR_SY		equ $e5
GAME_CUR_DX		equ $e4
GAME_CUR_DY		equ $e3
GAME_CUR_OFF	equ $e2
GAME_CUR_COL	equ $e1
GAME_ROTATE_X	equ $e0
GAME_ROTATE_Y	equ $df

GAME_SFX		equ $de
GAME_BGM		equ $dd
GAME_CODE		equ $d9 ;4 bytes
GAME_NTSC		equ $d8
GAME_LEVEL		equ $d7
GAME_ONLINE		equ $d5 ;word
GAME_TIME		equ $d4
GAME_ROTATE		equ $d3
GAME_TIME_DIV	equ $d2
GAME_RROT_CNT	equ $cf	;word
GAME_RROT_TIME	equ $cd	;word
GAME_TERM_CNT	equ $cc
GAME_TERM_OFF	equ $cb
GAME_CALL_MENU	equ $ca
GAME_TERM_TRACE	equ $c9
GAME_TRACE_CNT	equ $c8
GAME_ONLINE_SCR	equ $c7
GAME_TERM_FP	equ $c5	;word
GAME_DELAY		equ $c4
GAME_TABLE_VRAM	equ $c2	;word
GAME_TABLE_WDT	equ $c1
GAME_TABLE_HGT	equ $c0
GAME_TABLE_SRC	equ $be	;word
GAME_TABLE_CODE	equ $bd
GAME_TERM_ONGFX	equ $bc
GAME_TRACE_SKIP	equ $bb
GAME_TIME_OUT	equ $ba
GAME_ROTATE_SFX	equ $b9
GAME_START_POS	equ $b8
GAME_LOOP_ON	equ $b7

GAME_MAP		equ $300	;16x16 in memory, 12x12 on screen, contains current level
GAME_CHECK		equ $400	;16x16 in memory too, contains flags for active tiles
GAME_TERM_LIST	equ $500	;196b, contains offsets for terminals

GAME_MENU_BUF	equ $400	;140b, only on pause
GAME_MAP_BUF	equ $400	;256b, only on level init, used for level display effect

LEVELS_COUNT	equ 50
GAME_TIME_WARN	equ 5

SFX_CURSOR		equ $00
SFX_SELECT		equ $01
SFX_START		equ $02
SFX_TEXT		equ $03
SFX_ERROR		equ $04
SFX_ROTATE1		equ $05
SFX_ROTATE2		equ $06
SFX_ROTATE3		equ $07
SFX_ROTATE4		equ $08
SFX_LEVEL		equ $09
SFX_TIME		equ $0a
SFX_TIMEOUT		equ $0b
SFX_COUNT		equ $0c

BGM_NONE		equ $00
BGM_MENU		equ $01
BGM_GAME		equ $02
BGM_DONE		equ $03
BGM_TIMEOUT		equ $04

OP_RTI			equ $40	;RTI opcode
OP_JMP			equ $4c	;JMP opcode

NMI_EMPTY		equ 0
NMI_SOUND		equ 1
NMI_GAME		equ 2
NMI_DONE		equ 3

FT_BASE_ADR		= $0200	;page in RAM, should be $xx00
FT_TEMP			= $f0	;7 bytes in zeropage
FT_DPCM_OFF		= $e000	;$c000 or higher, 64-byte steps
FT_SFX_STREAMS	= 2		;number of sound effects played at once, can be 4 or less
FT_DPCM_PTR		= (FT_DPCM_OFF&$3fff)>>6

FT_DPCM_ENABLE			;undefine to exclude all the DMC code
FT_SFX_ENABLE			;undefine to exclude all the sound effects code
;FT_THREAD				;undefine if you call sound effects in the same thread as sound update

OAM_PAGE		equ $700



reset

;init hardware

    sei

    ldx #$40
    stx PPU_FRAMECNT
    ldx #$ff
    txs
    inx
    stx PPU_MASK
    stx DMC_FREQ
    stx PPU_CTRL		;no NMI

	jsr waitVBlank

    txa
clearRAM
    sta $000,x
    sta $100,x
    sta $200,x
    sta $300,x
    sta $400,x
    sta $500,x
    sta $600,x
    sta $700,x
    inx
    bne clearRAM

	lda #NMI_EMPTY
	jsr setNmiHandler
	lda #%10000000
	sta PPU_CTRL		;enable NMI

	jsr waitVBlank
	lda #$00
	sta PPU_SCROLL
	sta PPU_SCROLL

clearVRAM
	lda #$20
	sta PPU_ADDR
	lda #$00
	sta PPU_ADDR
	ldx #8
.1
	tay
.2
	sta PPU_DATA
	iny
	bne .2
	dex
	bne .1

	sta <FRAME_CNT
	sta <FRAME_CNT2

	jsr clearOAM
	jsr palReset
	jsr padInit


detectNTSC
	jsr waitNMI		;blargg's code
	ldx #52
	ldy #24
.1
	dex
	bne .1
	dey
	bne .1

	lda PPU_STATUS
	and #$80
	sta <GAME_NTSC


;init sound

	lda <GAME_NTSC
	jsr FamiToneInit

	ldx #LOW(bgm_game_dpcm)
	ldy #HIGH(bgm_game_dpcm)
	jsr FamiToneSampleInit

	ldx #LOW(sounds)
	ldy #HIGH(sounds)
	jsr FamiToneSfxInit

	lda #0
	sta $4017

;init game

initGame
	lda #1
	sta <GAME_SFX
	sta <GAME_BGM
	lda #0
	sta <GAME_CODE
	sta <GAME_CODE+1
	sta <GAME_CODE+2
	sta <GAME_CODE+3

	lda #0
	ldx #0
.1
	sta GAME_MAP,x
	inx
	bne .1

	lda #1
	sta <RAND_SEED

mainLoop
	jsr mainMenu
	lda <GAME_LEVEL
	cmp #LEVELS_COUNT
	beq .done
	jsr gamePlayInit
	lda <GAME_LEVEL
	cmp #LEVELS_COUNT
	bne mainLoop
.done
	jmp wellDone


levelClear
	jsr waitNMI
	lda #NMI_EMPTY
	jsr setNmiHandler

	jsr waitNMI
	lda #%00001110	;enable display, disable sprites
	sta PPU_MASK
	lda #SFX_LEVEL
	jsr sfxPlay
	jsr FamiToneUpdate

	lda #0
	sta <GAME_TERM_OFF
	lda #50
	sta <GAME_DELAY
	lda #8
	sta <GAME_TERM_ONGFX

.update
	jsr waitNMI50

	lda <FRAME_CNT
	rol a
	rol a
	and #%00010000
	ora #%10000000
	sta PPU_CTRL

	jsr updateStats

	ldx #4
.1
	txa
	pha
	ldx <GAME_TERM_OFF
	cpx <GAME_TERM_CNT
	beq .2
	lda GAME_TERM_LIST,x
	jsr getTileCoords
	jsr updateTile
	inc <GAME_TERM_OFF
.2
	pla
	tax
	dex
	bne .1

	jsr resetPPUAdr

	jsr FamiToneUpdate

	lda <GAME_ONLINE_SCR
	cmp #100
	bcs .3
	clc
	adc #2
	cmp #100
	bcc .3
	lda #100
.3
	sta <GAME_ONLINE_SCR

	dec <GAME_DELAY
	bne .update

	inc <GAME_LEVEL

	lda #$67
	sta <GAME_TABLE_VRAM
	lda #$21
	sta <GAME_TABLE_VRAM+1
	lda #LOW(levelClearTable)
	sta <GAME_TABLE_SRC
	lda #HIGH(levelClearTable)
	sta <GAME_TABLE_SRC+1
	ldx #18
	ldy #8
	lda #1
	jsr showTable

.wait
	jsr waitNMI50
	jsr FamiToneUpdate
	jsr padPoll
	lda <PAD_STATET
	and #PAD_A|PAD_B|PAD_START
	beq .wait

	lda <GAME_LEVEL
	cmp #LEVELS_COUNT
	bne .next
	jmp exitGame

.next
	jsr clearTable

	lda #0
	jsr showLevel

	jmp gamePlay


timeOut
	jsr waitNMI
	lda #NMI_EMPTY
	jsr setNmiHandler

	jsr waitNMI
	lda #%00001110	;enable display, disable sprites
	sta PPU_MASK
	lda #0			;make timer visible after blinking
	sta <GAME_TIME_DIV
	jsr updateStats
	jsr resetPPUAdr

	lda #BGM_NONE
	jsr bgmPlay
	lda #SFX_TIMEOUT
	jsr sfxPlay
	jsr FamiToneUpdate

	jsr traceMap
	bne .1
	jmp levelClear
.1
	lda #50
	sta <TEMP
.2
	jsr waitNMI50
	jsr FamiToneUpdate
	dec <TEMP
	bne .2

	jsr waitNMI
	lda #$67
	sta <GAME_TABLE_VRAM
	lda #$21
	sta <GAME_TABLE_VRAM+1
	lda #LOW(timeOutTable)
	sta <GAME_TABLE_SRC
	lda #HIGH(timeOutTable)
	sta <GAME_TABLE_SRC+1
	ldx #18
	ldy #8
	lda #0
	jsr showTable

	jsr FamiToneUpdate

	lda #BGM_TIMEOUT
	jsr bgmPlay

.wait
	jsr waitNMI50
	jsr FamiToneUpdate
	jsr padPoll
	lda <PAD_STATET
	and #PAD_A|PAD_B|PAD_START
	beq .wait

exitGame
	lda #BGM_NONE
	jsr bgmPlay

	ldx #LOW(palGame)
	ldy #HIGH(palGame)
	jsr palFadeOut

	rts				;exit from game


timeOutCur
	ldx #$80
	ldy #$80
	lda <MENU_CNT
	and #16
	bne .2
	lda <MENU_CUR
	bne .1
	ldx #$47
	jmp .2
.1
	ldy #$47
.2
	lda #$22
	sta PPU_ADDR
	lda #$0a
	sta PPU_ADDR
	stx PPU_DATA
	lda #$22
	sta PPU_ADDR
	lda #$4a
	sta PPU_ADDR
	sty PPU_DATA
	rts

timeOutCurAttr
	sta <TEMP
	asl a
	adc <TEMP
	tax
	lda #$23
	sta PPU_ADDR
	lda #$e3
	sta PPU_ADDR
	lda timeOutAttrTable,x
	sta PPU_DATA
	lda timeOutAttrTable+1,x
	sta PPU_DATA
	lda timeOutAttrTable+2,x
	sta PPU_DATA
	rts

timeOutAttrTable
	.db $f0,$f0,$30
	.db $0f,$0f,$03
	.db $ff,$ff,$33

gamePlayInit
	jsr waitNMI
	lda #0
	sta PPU_MASK

	jsr initLevelVars

	ldx #LOW(gameBgNameTable)
	ldy #HIGH(gameBgNameTable)
	lda #$20
	sta PPU_ADDR
	lda #$00
	sta PPU_ADDR
	jsr unrle
	jsr updateStats

	jsr clearOAM
	jsr showCursor

	jsr waitNMI
	lda #%00011110	;enable display, enable sprites
	sta PPU_MASK
	jsr updateOAM

	lda #BGM_GAME
	jsr bgmPlay

	ldx #LOW(palGame)
	ldy #HIGH(palGame)
	jsr palFadeIn

gamePlay
	lda <GAME_LEVEL
	beq .noPass

	clc
	adc #-1
	asl a
	tax
	inx
	lda passwords,x
	ror a
	ror a
	ror a
	ror a
	and #15
	sta <GAME_CODE
	lda passwords,x
	and #15
	sta <GAME_CODE+1
	dex
	lda passwords,x
	ror a
	ror a
	ror a
	ror a
	and #15
	sta <GAME_CODE+2
	lda passwords,x
	and #15
	sta <GAME_CODE+3

.noPass
	jsr initLevelVars
	jsr waitNMI
	jsr FamiToneUpdate
	jsr unpackLevel

	jsr waitNMI
	jsr updateStats
	jsr resetPPUAdr
	jsr FamiToneUpdate

	lda #1
	jsr showLevel
	jmp gameLoopInit

restartLevel
	jsr initLevelVars
	lda #%00001110	;enable display, disable sprites
	sta PPU_MASK

	jsr waitNMI
	jsr updateStats
	jsr resetPPUAdr

	jsr FamiToneUpdate

	lda #3
	sta <GAME_DELAY
.rand0
	jsr waitNMI
	jsr FamiToneUpdate
	jsr unpackLevel

	lda #0
	sta <GAME_ROTATE_Y
.rand2
	jsr waitNMI

	lda #0
	sta <GAME_ROTATE_X
.rand3
	ldx <GAME_ROTATE_X
	ldy <GAME_ROTATE_Y
	jsr updateTile
	inc <GAME_ROTATE_X
	lda <GAME_ROTATE_X
	cmp #6
	bne .rand3

	jsr resetPPUAdr
	jsr FamiToneUpdate

	jsr waitNMI

.rand4
	ldx <GAME_ROTATE_X
	ldy <GAME_ROTATE_Y
	jsr updateTile
	inc <GAME_ROTATE_X
	lda <GAME_ROTATE_X
	cmp #12
	bne .rand4

	jsr resetPPUAdr
	jsr FamiToneUpdate

	inc <GAME_ROTATE_Y
	lda <GAME_ROTATE_Y
	cmp #12
	bne .rand2

	dec <GAME_DELAY
	bne .rand0

	
gameLoopInit
	lda #NMI_SOUND
	jsr setNmiHandler
	jsr traceMap
	lda #NMI_EMPTY
	jsr setNmiHandler
	lda #0
	sta <GAME_CALL_MENU
	sta <GAME_TRACE_CNT

	jsr clearOAM
	jsr showCursor

	jsr waitNMI
	lda #%00011110	;enable display, enable sprites
	sta PPU_MASK
	jsr updateOAM
	jsr FamiToneUpdate

	lda #1
	sta <GAME_LOOP_ON
	jsr waitNMI
	lda #NMI_GAME
	jsr setNmiHandler

	
gameLoop
	lda <GAME_TRACE_CNT
	bne .1
	lda #25
	sta <GAME_TRACE_CNT
	lda #0
	sta <GAME_TRACE_SKIP
	jsr traceMap
	bne .1
	lda <GAME_TRACE_SKIP
	bne .1
	sta <GAME_LOOP_ON
	jmp levelClear
.1
	lda <GAME_TIME_OUT
	beq .2
	lda #0
	sta <GAME_LOOP_ON
	jmp timeOut
.2
	lda <GAME_CALL_MENU
	beq gameLoop

	lda #0
	sta <GAME_LOOP_ON
	jsr waitNMI
	lda #NMI_EMPTY
	jsr setNmiHandler
	jmp gameMenu

	
gameLoopCode
	lda <GAME_LOOP_ON
	bne .on
	rts
.on
	lda <FRAME_CNT
	and #%00010000
	ora #%10000000
	sta PPU_CTRL

	jsr updateOAM

	lda #$3f
	sta PPU_ADDR
	lda #$11
	sta PPU_ADDR
	lda <GAME_CUR_COL
	lsr a
	lsr a
	tax
	lda curColors,x
	sta PPU_DATA

	jsr updateStats

rotateTile
	lda <GAME_ROTATE
	bne .rotate
	jmp .noRotate
.rotate
	ldx <GAME_ROTATE_X
	ldy <GAME_ROTATE_Y
	jsr getTileAddr

	lda GAME_MAP,x
	stx <TEMP
	sta <TEMP+1
.1
	tax
	lda rotateLeft,x
	dec <GAME_ROTATE
	bne .1

	cmp <TEMP+1
	beq .4				;only update the tile if it is rotateable
	ldx <TEMP
	sta GAME_MAP,x

	ldx <GAME_ROTATE_X
	ldy <GAME_ROTATE_Y
	jsr updateTile

	lda #12*4-1
	sta <GAME_CUR_COL

	lda <GAME_ROTATE_SFX
	jsr sfxPlay

	lda #0				;force trace to minimize time of level clear detection
	sta <GAME_TRACE_CNT
	lda #1				;set on-going trace result to non-valid, because map was modified
	sta <GAME_TRACE_SKIP
.4

.noRotate

updateTerminals
	ldx <GAME_TERM_OFF
	lda GAME_TERM_LIST,x
	jsr getTileCoords
	jsr updateTile
	inc <GAME_TERM_OFF
	lda <GAME_TERM_OFF
	cmp <GAME_TERM_CNT
	bne .1
	lda #0
	sta <GAME_TERM_OFF
.1
	jsr resetPPUAdr


randomRotateTile
	lda <GAME_RROT_TIME
	ora <GAME_RROT_TIME+1
	beq .1
	lda <GAME_RROT_CNT
	ora <GAME_RROT_CNT+1
	bne .count
	jsr setRandomRotate
	jsr rand
	and #$7f
	adc <GAME_RROT_TIME
	sta <GAME_RROT_CNT
	lda <GAME_RROT_TIME+1
	adc #0
	sta <GAME_RROT_CNT+1
.count
	dec <GAME_RROT_CNT
	bne .1
	lda <GAME_RROT_CNT+1
	beq .1
	dec <GAME_RROT_CNT+1
.1

updateStatOnline
	lda <GAME_ONLINE_SCR
	cmp <GAME_ONLINE+1
	beq .2
	bcc .1
	dec <GAME_ONLINE_SCR
	jmp .2
.1
	inc <GAME_ONLINE_SCR
.2
	lda <GAME_ONLINE_SCR
	cmp #100
	bcc .3
	lda #100
	sta <GAME_ONLINE_SCR
.3

moveCursor
	lda <GAME_CUR_OFF
	beq .noMove
	lda <GAME_CUR_SX
	clc
	adc <GAME_CUR_DX
	sta <GAME_CUR_SX
	lda <GAME_CUR_SY
	clc
	adc <GAME_CUR_DY
	sta <GAME_CUR_SY
	dec <GAME_CUR_OFF
	bne .noMove
	lda #0
	sta <GAME_CUR_DX
	sta <GAME_CUR_DY
.noMove
	dec <GAME_CUR_COL
	bpl .done
	lda #8*4-1
	sta <GAME_CUR_COL
.done
	jsr showCursor

	jsr padPoll

	lda <PAD_STATET
	and #PAD_START
	beq .noStart
	lda <GAME_CUR_OFF
	bne .noStart
	lda #1
	sta <GAME_CALL_MENU
	rts
.noStart

	lda <PAD_STATET
	and #PAD_B
	beq .rot1
	ldx #SFX_ROTATE1
	lda #3
	jmp setRotate
.rot1
	lda <PAD_STATET
	and #PAD_A
	beq .rot2
	ldx #SFX_ROTATE2
	lda #1
	jmp setRotate
.rot2
	lda <PAD_STATET
	and #PAD_SELECT
	beq .rot3
	ldx #SFX_ROTATE1
	lda #2
	jmp setRotate
.rot3

	ldx #0
	ldy #0

	lda <PAD_STATE
	and #PAD_UP
	beq .move1
	lda <GAME_CUR_SY
	cmp #31
	beq .move1
	ldy #-2
.move1
	lda <PAD_STATE
	and #PAD_DOWN
	beq .move2
	lda <GAME_CUR_SY
	cmp #31+11*16
	beq .move2
	ldy #2
.move2
	lda <PAD_STATE
	and #PAD_LEFT
	beq .move3
	lda <GAME_CUR_SX
	cmp #32
	beq .move3
	ldx #-2
.move3
	lda <PAD_STATE
	and #PAD_RIGHT
	beq .move4
	lda <GAME_CUR_SX
	cmp #32+11*16
	beq .move4
	ldx #2
.move4
	stx <TEMP
	tya
	ora <TEMP
	bne setCurMove

	rts


setCurMove
	lda <GAME_CUR_OFF
	bne .1
	stx <GAME_CUR_DX
	sty <GAME_CUR_DY
	lda #8
	sta <GAME_CUR_OFF
.1
	rts


setRotate
	sta <GAME_ROTATE
	stx <GAME_ROTATE_SFX

	lda <GAME_CUR_SX
	clc
	adc #-32
	ror a
	ror a
	ror a
	ror a
	and #15
	sta <GAME_ROTATE_X
	lda <GAME_CUR_DX		;if cursor moves, rotate destination tile
	beq .1
	bmi .1
	inc <GAME_ROTATE_X
.1
	lda <GAME_CUR_SY
	clc
	adc #-31
	ror a
	ror a
	ror a
	ror a
	and #15
	sta <GAME_ROTATE_Y
	lda <GAME_CUR_DY
	beq .2
	bmi .2
	inc <GAME_ROTATE_Y
.2
	rts


setRandomRotate
	jsr rand
	tax
	lda GAME_MAP,x
	beq setRandomRotate
	txa
	and #$0f
	sta <GAME_ROTATE_X
	dec <GAME_ROTATE_X
	txa
	ror a
	ror a
	ror a
	ror a
	and #$0f
	sta <GAME_ROTATE_Y
	dec <GAME_ROTATE_Y
	jsr rand
	and #2
	ora #1
	sta <GAME_ROTATE
	ldx #SFX_ROTATE3
	cmp #1
	beq .1
	inx
.1
	stx <GAME_ROTATE_SFX
	rts


;update cursor sprites in OAM buffer

showCursor
	ldy #0
	lda <GAME_CUR_SY
	sta OAM_PAGE+0
	lda #$bf
	sta OAM_PAGE+1
	sty OAM_PAGE+2
	lda <GAME_CUR_SX
	sta OAM_PAGE+3

	lda <GAME_CUR_SY
	sta OAM_PAGE+4
	lda #$c0
	sta OAM_PAGE+5
	sty OAM_PAGE+6
	lda <GAME_CUR_SX
	clc
	adc #8
	sta OAM_PAGE+7

	lda <GAME_CUR_SY
	clc
	adc #8
	sta OAM_PAGE+8
	lda #$cb
	sta OAM_PAGE+9
	sty OAM_PAGE+10
	lda <GAME_CUR_SX
	sta OAM_PAGE+11

	lda <GAME_CUR_SY
	clc
	adc #8
	sta OAM_PAGE+12
	lda #$cc
	sta OAM_PAGE+13
	sty OAM_PAGE+14
	lda <GAME_CUR_SX
	clc
	adc #8
	sta OAM_PAGE+15

	rts


updateStats
	lda #$20
	sta PPU_ADDR
	lda #$49
	sta PPU_ADDR
	lda <GAME_LEVEL
	clc
	adc #1
	jsr putNum2

	lda #$20
	sta PPU_ADDR
	lda #$50
	sta PPU_ADDR
	lda <GAME_ONLINE_SCR
	jsr putNum3

	lda #$20
	sta PPU_ADDR
	lda #$59
	sta PPU_ADDR
	lda <GAME_TIME
	cmp #GAME_TIME_WARN
	bcs .1
	lda <GAME_TIME_DIV
	and #$20
	beq .1
	lda #0
	sta PPU_DATA
	sta PPU_DATA
	sta PPU_DATA
	rts
.1
	lda <GAME_TIME
	jsr putNum3

	rts


putNum3
	ldx #$56
.1
	inx
	clc
	adc #-100
	bcs .1

	stx PPU_DATA
	adc #100

putNum2
	ldx #$56
.2
	inx
	clc
	adc #-10
	bcs .2

	stx PPU_DATA
	adc #10+$57
	sta PPU_DATA
	rts


showTable
	stx <GAME_TABLE_WDT
	sty <GAME_TABLE_HGT
	sty <TEMP+3 ;y
	sta <GAME_TABLE_CODE
	lda #0
	sta <TEMP+2	;offset
	lda <GAME_TABLE_VRAM
	sta <TEMP+4
	lda <GAME_TABLE_VRAM+1
	sta <TEMP+5
.tableshow0
	jsr waitNMI50
	lda <TEMP+5
	sta PPU_ADDR
	lda <TEMP+4
	sta PPU_ADDR
	lda PPU_DATA

	ldy <GAME_TABLE_WDT
	ldx <TEMP+2
.tableshow1
	lda PPU_DATA
	sta GAME_MENU_BUF,x
	inx
	dey
	bne .tableshow1

	lda <TEMP+5
	sta PPU_ADDR
	lda <TEMP+4
	sta PPU_ADDR
	ldx <GAME_TABLE_WDT
	ldy <TEMP+2
.tableshow2
	lda [GAME_TABLE_SRC],y
	sta PPU_DATA
	iny
	dex
	bne .tableshow2
	sty <TEMP+2

	lda <GAME_TABLE_CODE
	beq .tableshow3
	cpy #6*18
	bne .tableshow3
	lda #$22
	sta PPU_ADDR
	lda #$11
	sta PPU_ADDR
	jsr showPassCode
.tableshow3

	jsr resetPPUAdr
	jsr FamiToneUpdate

	lda <TEMP+4
	clc
	adc #32
	sta <TEMP+4
	lda <TEMP+5
	adc #0
	sta <TEMP+5

	dec <TEMP+3
	bne .tableshow0

	rts


clearTable
	lda <GAME_TABLE_VRAM
	sta <TEMP
	lda <GAME_TABLE_VRAM+1
	sta <TEMP+1
	lda #0
	sta <TEMP+2	;offset
.tablehide0
	jsr waitNMI50
	lda <TEMP+1
	sta PPU_ADDR
	lda <TEMP
	sta PPU_ADDR

	ldy <GAME_TABLE_WDT
	ldx <TEMP+2
.tablehide1
	lda GAME_MENU_BUF,x
	sta PPU_DATA
	inx
	dey
	bne .tablehide1
	stx <TEMP+2

	jsr resetPPUAdr
	jsr FamiToneUpdate

	lda <TEMP
	clc
	adc #32
	sta <TEMP
	lda <TEMP+1
	adc #0
	sta <TEMP+1

	dec <GAME_TABLE_HGT
	bne .tablehide0

	rts

;print passcode of current level to VRAM

showPassCode
	lda <GAME_LEVEL
	bne .show
	rts

.show
	clc
	adc #-1
	asl a
	tax
	inx
	ldy #2
.pass1
	lda passwords,x
	ror a
	ror a
	ror a
	ror a
	and #15
	clc
	adc #$57
	sta PPU_DATA
	lda passwords,x
	and #15
	clc
	adc #$57
	sta PPU_DATA
	dex
	dey
	bne .pass1
	rts


initLevelVars
	lda #32+5*16
	sta <GAME_CUR_SX
	lda #31+5*16
	sta <GAME_CUR_SY

	lda #0
	sta <GAME_CUR_DX
	sta <GAME_CUR_DY
	sta <GAME_CUR_OFF
	sta <GAME_ROTATE
	sta <GAME_TIME_DIV
	sta <GAME_ONLINE
	sta <GAME_ONLINE+1
	sta <GAME_ONLINE_SCR
	sta <GAME_TERM_TRACE
	sta <GAME_RROT_TIME
	sta <GAME_RROT_TIME+1
	sta <GAME_RROT_CNT
	sta <GAME_RROT_CNT+1
	sta <GAME_TRACE_SKIP
	sta <GAME_TIME_OUT
	lda #1
	sta <GAME_CUR_COL
	lda #4
	sta <GAME_TERM_ONGFX

	lda <GAME_LEVEL
	asl a
	tax
	lda levList,x
	sta <TEMP
	lda levList+1,x
	sta <TEMP+1
	ldy #58
	lda [TEMP],y
	sta <GAME_TIME

	rts


;displays current level
;in: A is 1 to display, 0 to clear

showLevel
	sta <TEMP+7
	ldx #0
	stx <TEMP+8
	lda #$ff
	sta <TEMP+9
.resetbuf
	lda GAME_MAP,x
	sta GAME_MAP_BUF,x
	inx
	bne .resetbuf

.setstart
	ldx <GAME_START_POS
	lda #$ff
	sta GAME_MAP_BUF,x

	lda #$ff
	sta <TEMP+6		;code to mark next tiles

.loop
	lda #0
	sta <TEMP+4		;loop counter
	sta <TEMP+5		;set if new tiles were added
.loop0
	ldx <TEMP+4
	lda GAME_MAP_BUF,x
	beq .next1		;skip tile if it is empty
	cmp <TEMP+6
	bne .next1		;or it is not current

	lda #0			;mark visited tile as empty
	sta GAME_MAP_BUF,x

	lda <TEMP+7
	bne .loop1
	sta GAME_MAP,x
.loop1

	lda <TEMP+4		;get tile coords
	and #15
	tax
	lda <TEMP+4
	ror a
	ror a
	ror a
	ror a
	and #15
	tay
	dex
	dey

	lda <TEMP+8
	eor #1
	sta <TEMP+8
	beq .skip2		;display even, store odd
	jsr waitNMI50	;update tile
	jsr updateTile
	ldx <TEMP+9
	cpx #$ff
	beq .skip1
	ldy <TEMP+10
	jsr updateTile
.skip1
	jsr resetPPUAdr
	jsr FamiToneUpdate
	jmp .skip3
.skip2
	stx <TEMP+9
	sty <TEMP+10
.skip3

	ldx <TEMP+4
	lda <TEMP+6
	eor #$01
	tay

	lda GAME_MAP_BUF-1,x	;check tile on the left
	beq .noLeft
	tya
	sta GAME_MAP_BUF-1,x	;if there is a tile, mark it as next
	sta <TEMP+5				;and set new tiles flag
.noLeft
	lda GAME_MAP_BUF+1,x	;check tile on the right
	beq .noRight
	tya
	sta GAME_MAP_BUF+1,x
	sta <TEMP+5
.noRight
	lda GAME_MAP_BUF-16,x	;check tile above
	beq .noUp
	tya
	sta GAME_MAP_BUF-16,x
	sta <TEMP+5
.noUp
	lda GAME_MAP_BUF+16,x	;check tile below
	beq .noDown
	tya
	sta GAME_MAP_BUF+16,x
	sta <TEMP+5
.noDown

.next1
	inc <TEMP+4
	beq .next2
	jmp .loop0
.next2

	lda <TEMP+6
	eor #$01
	sta <TEMP+6

	lda <TEMP+5		;repeat until no tiles added
	beq .done
	jmp .loop
.done

	lda <TEMP+8		;display stored if even number of tiles
	bne .noEven
	jsr waitNMI50	;update tile
	ldx <TEMP+9
	ldy <TEMP+10
	jsr updateTile
	jsr resetPPUAdr
	jsr FamiToneUpdate
.noEven
	rts


;unpack current level into GAME_MAP, randomize it, build list of terminals offsets

unpackLevel
	lda #NMI_SOUND
	jsr setNmiHandler

	lda <GAME_LEVEL
	asl a
	tax
	lda levList,x
	sta <TEMP
	lda levList+1,x
	sta <TEMP+1

	ldx #16+1
	ldy #0
	sty <TEMP+7
.1
	lda [TEMP],y
	iny
	sta <TEMP+2
	lda [TEMP],y
	iny
	sta <TEMP+3
	lda [TEMP],y
	iny
	sta <TEMP+4
	lda #8
	sta <TEMP+5
.2
	lda <TEMP+2
	and #7
	stx <TEMP+6
	tax
	lda mapCodes,x

	tax
	jsr rand		;random rotation for all the elements
	and #3
	clc
	adc #1
	sta <TEMP+8
.3
	lda rotateLeft,x
	tax
	dec <TEMP+8
	bne .3

	ldx <TEMP+6
	sta GAME_MAP,x
	inx

	inc <TEMP+7
	lda <TEMP+7
	cmp #12
	bne .4
	lda #0
	sta <TEMP+7
	inx
	inx
	inx
	inx
.4

	ror <TEMP+4
	ror <TEMP+3
	ror <TEMP+2
	ror <TEMP+4
	ror <TEMP+3
	ror <TEMP+2
	ror <TEMP+4
	ror <TEMP+3
	ror <TEMP+2

	dec <TEMP+5
	bne .2

	cpy #54
	bne .1

	lda [TEMP],y
	iny
	sta <GAME_TERM_FP
	lda [TEMP],y
	iny
	sta <GAME_TERM_FP+1
	lda [TEMP],y
	iny
	sta <GAME_RROT_TIME
	sta <GAME_RROT_CNT
	lda [TEMP],y
	iny
	sta <GAME_RROT_TIME+1
	sta <GAME_RROT_CNT+1
	lda [TEMP],y
	iny
	sta <GAME_TIME
	lda [TEMP],y
	sta <GAME_START_POS

	ldx #0
	ldy #0
.5
	lda GAME_MAP,x
	cmp #15
	bcc .6
	txa
	sta GAME_TERM_LIST,y
	iny
.6
	inx
	bne .5

	sty <GAME_TERM_CNT
	stx <GAME_TERM_OFF

.disconnect0		;now rotate terminals to make them disconnected
	lda GAME_TERM_LIST,x
	stx <TEMP
	ldx #0
	stx <TEMP+1
	tay
.disconnect1
	lda GAME_MAP,y
	tax
	lda checkTerminalConnection,x
	beq .disconnect1
	clc
	ldx <TEMP
	adc GAME_TERM_LIST,x
	tax
	lda GAME_MAP,x
	beq .disconnect2
	lda GAME_MAP,y
	tax
	lda rotateLeft,x
	sta GAME_MAP,y
	inc <TEMP+1		;don't rotate a terminal more than 3 times
	lda <TEMP+1
	cmp #4
	bne .disconnect1
.disconnect2
	ldx <TEMP
	inx
	cpx <GAME_TERM_CNT
	bne .disconnect0

	lda #NMI_EMPTY
	jsr setNmiHandler

	rts


;get address of a tile with given coords in GAME_MAP
;in:  X,Y coords of a tile 0..11
;out: X offset for GAME_MAP

getTileAddr
	stx <TEMP+2
	tya			;y*16+x+17
	asl a
	asl a
	asl a
	asl a
	ora <TEMP+2
	adc #16+1	;with Y <15 clc is not needed
	tax
	rts


;get tile coords in X,Y for given offset in GAME_MAP
;in:  A offset
;out: X,Y coords

getTileCoords
	tay
	and #$0f
	tax
	tya
	ror a
	ror a
	ror a
	ror a
	and #$0f
	tay
	dex
	dey
	rts

	
;update a tile with given coords in the nametable
;in: X,Y coords of a tile 0..11

updateTile
	stx <TEMP
	sty <TEMP+1

	jsr getTileAddr
	lda GAME_MAP,x	;get the tile from the map
	cmp #15
	bcc .1
	ldy <GAME_ONLINE_SCR
	beq .1
	ldy GAME_CHECK,x
	beq .1
	clc
	adc <GAME_TERM_ONGFX
.1
	asl a
	asl a
	tax				;offset in the tiles table

	lda #0			;get nametable address, y*64+x*2
	sta <TEMP+2
	clc
	ror <TEMP+1
	ror <TEMP+2
	clc
	ror <TEMP+1
	ror <TEMP+2		;H +1, L +2

	lda <TEMP
	asl a
	adc #$84		;with X<15 clc is not needed
	sta <TEMP
	clc
	adc <TEMP+2
	tay
	lda <TEMP+1
	adc #$20
	sta <TEMP+1
	sta PPU_ADDR
	sty PPU_ADDR

	lda tilesTable,x
	sta PPU_DATA
	lda tilesTable+1,x
	sta PPU_DATA

	tya
	clc
	adc #$20
	tay
	lda <TEMP+1
	adc #0
	sta PPU_ADDR
	sty PPU_ADDR

	lda tilesTable+2,x
	sta PPU_DATA
	lda tilesTable+3,x
	sta PPU_DATA

	rts


;play sfx using FamiTone, always using the same channel
;in: A sound effect number

sfxPlay
	tay
	lda <GAME_SFX
	and #$01
	bne .1
	rts
.1
	tya
	ldx #FT_SFX_CH0
	cmp #SFX_TIME
	bne .2
	ldx #FT_SFX_CH1
.2
	jmp FamiToneSfxStart


;play music using FamiTone
;in: A music number

bgmPlay
	tax
	lda <GAME_BGM
	and #1
	beq .1
	txa
.1
	bne .2
	jmp FamiToneMusicStop
.2
	cmp #BGM_MENU
	bne .3
	ldx #LOW(bgm_title_module)
	ldy #HIGH(bgm_title_module)
	jmp FamiToneMusicStart
.3
	cmp #BGM_GAME
	bne .4
	ldx #LOW(bgm_game_module)
	ldy #HIGH(bgm_game_module)
	jmp FamiToneMusicStart
.4
	cmp #BGM_DONE
	bne .5
	ldx #LOW(bgm_done_module)
	ldy #HIGH(bgm_done_module)
	jmp FamiToneMusicStart
.5
	cmp #BGM_TIMEOUT
	bne .6
	ldx #LOW(bgm_timeout_module)
	ldy #HIGH(bgm_timeout_module)
	jmp FamiToneMusicStart
.6
	rts

;Galois random generator, found somewhere
;out: A random number 0..255

rand
	lda <RAND_SEED
	asl a
	bcc .1
	eor #$cf
.1
	sta <RAND_SEED
	rts


resetPPUAdr
	lda #0
	sta PPU_ADDR
	sta PPU_ADDR
	rts


waitVBlank
    bit PPU_STATUS
.1
    bit PPU_STATUS
    bpl .1
	rts


waitNMI
	lda <FRAME_CNT
.1
	cmp <FRAME_CNT
	beq .1
	rts


;if in NTSC mode, counts frames and returns C=1 every 6th frame

ntscIsSkip
	lda <GAME_NTSC
	beq .1
	inc <FRAME_CNT2
	lda <FRAME_CNT2
	cmp #5
	bne .1
	lda #0
	sta <FRAME_CNT2
	sec
	rts
.1
	clc
	rts


waitNMI50
	lda <FRAME_CNT
.1
	cmp <FRAME_CNT
	beq .1
	jsr ntscIsSkip
	bcc .2
	txa
	pha
	tya
	pha
	jsr FamiToneUpdate
	pla
	tay
	pla
	tax
	jmp waitNMI50
.2
	rts


clearOAM
	ldx #0
	lda #$ef
.1
	sta OAM_PAGE,x
	inx
	inx
	inx
	inx
	bne .1

	rts
	
	
updateOAM
	lda #0
	sta PPU_OAM_ADDR
	lda #HIGH(OAM_PAGE)
	sta PPU_OAM_DMA
	rts


;set NMI handler by it's number in NMI handlers list
;in: A is number of needed handler

setNmiHandler
	asl a
	tax
	lda nmiHandlersList,x
	ldy nmiHandlersList+1,x
	ldx #OP_RTI
	stx <NMI_CALL
	sta <NMI_CALL+1
	sty <NMI_CALL+2
	lda #OP_JMP
	sta <NMI_CALL
	rts


;empty NMI handler

nmiEmpty
	inc <FRAME_CNT
	rti


;empty NMI handler with sound update

nmiSound
	inc <FRAME_CNT
	pha
	txa
	pha
	tya
	pha
	jsr FamiToneUpdate
	pla
	tay
	pla
	tax
	pla
	rti


;game NMI handler

nmiGame
	inc <FRAME_CNT
	pha
	txa
	pha
	tya
	pha

	jsr ntscIsSkip
	bcs .3


	inc <GAME_TIME_DIV
	lda <GAME_TIME_DIV
	cmp #128
	bne .1
	lda #0
	sta <GAME_TIME_DIV
	lda <GAME_TIME
	beq .noTimeDec
	cmp #GAME_TIME_WARN+1
	bcs .noTimeSfx
	lda #SFX_TIME
	jsr sfxPlay
.noTimeSfx
	dec <GAME_TIME
	jmp .1
.noTimeDec
	lda #1
	sta <GAME_TIME_OUT
.1
	lda <GAME_TRACE_CNT
	beq .2
	dec <GAME_TRACE_CNT
.2
	jsr gameLoopCode

.3
	jsr FamiToneUpdate

	pla
	tay
	pla
	tax
	pla
	rti


nmiHandlersList
	.dw nmiEmpty,nmiSound,nmiGame,nmiDone


	.include "mainmenu.asm"
	.include "gamemenu.asm"
	.include "tracemap.asm"
	.include "rle.asm"
	.include "palette.asm"
	.include "controller.asm"
	.include "famitone.asm"

	.bank 1
	.org $a000

	.include "bgm_title.asm"
	.include "bgm_done.asm"
	.include "bgm_timeout.asm"

    .bank 2
	.org $c000

	.include "bgm_game.asm"
	.include "levels.asm"

	.bank 3
	.org $e000

	.incbin  "bgm_game_dpcm.bin"
	.include "bgm_game_dpcm.asm"
	.include "sfx.asm"
	.include "welldone.asm"

titleNameTable
	.incbin "title.rle"

gameBgNameTable
	.incbin "gamebg.rle"

tilesTable
	.db $8d,$8e		;0 empty
	.db $a1,$a2

	.db $8f,$90		;1 horizontal
	.db $a3,$a4

	.db $91,$92		;2 vertical
	.db $a5,$a6

	.db $93,$94		;3 crossed separate
	.db $a7,$a8

	.db $95,$96		;4 crossed connection
	.db $a9,$aa

	.db $97,$98		;5 left-down
	.db $a9,$a6

	.db $99,$9a		;6 left-up
	.db $ab,$a2

	.db $9b,$96		;7 right-up
	.db $ac,$a4

	.db $9c,$90		;8 right-down
	.db $a5,$aa

	.db $9d,$90		;9 left-right-down
	.db $a9,$aa

	.db $9e,$92		;10 left-up-down
	.db $a9,$a6

	.db $9f,$96		;11 left-right-up
	.db $a3,$a4

	.db $ae,$96		;12 right-up-down
	.db $a5,$aa

	.db $af,$b0		;13 left-up and right-down
	.db $c1,$c2

	.db $b1,$b2		;14 left-down and right-up
	.db $c3,$c4

	.db $b3,$b4		;15 terminal right (offline)
	.db $c5,$c6

	.db $b3,$b5		;16 terminal down
	.db $c7,$c8

	.db $b6,$b5		;17 terminal left
	.db $c9,$ca

	.db $b7,$b8		;18 terminal up
	.db $c5,$ca

	.db $b9,$ba		;19 terminal right (connecting)
	.db $c5,$c6

	.db $b9,$bb		;20 terminal down
	.db $c7,$c8

	.db $bc,$bb		;21 terminal left
	.db $c9,$ca

	.db $bd,$be		;22 terminal up
	.db $c5,$ca

	.db $d1,$d2		;19 terminal right (online)
	.db $c5,$c6

	.db $d1,$d3		;20 terminal down
	.db $c7,$c8

	.db $d4,$d3		;21 terminal left
	.db $c9,$ca

	.db $d5,$d6		;22 terminal up
	.db $c5,$ca

mapCodes
	.db 0,1,3,4,5,9,13,15

rotateLeft
	.db 0,2,1,3,4,6,7,8,5,10,11,12,9,14,13,16,17,18,15

checkTerminalConnection
	.db 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,16,-1,-16

curColors
	.db $21,$21,$2c,$2c,$2b,$2b,$2c,$2c
	.db $38,$39,$37,$30

levelClearTable
	.db $e0,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e2
	.db $e3,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$e4
	.db $e3,$00,$ea,$81,$81,$80,$ec,$84,$85,$85,$82,$ec,$87,$82,$ed,$f0,$00,$e4
	.db $e3,$00,$00,$80,$80,$80,$80,$80,$80,$80,$80,$80,$80,$80,$80,$00,$00,$e4
	.db $e3,$00,$00,$80,$80,$80,$80,$80,$80,$80,$80,$80,$80,$80,$80,$00,$00,$e4
	.db $e3,$00,$00,$80,$ec,$84,$ed,$82,$80,$80,$ee,$ee,$ee,$ee,$80,$00,$00,$e4
	.db $e3,$00,$00,$80,$80,$80,$80,$80,$80,$80,$80,$80,$80,$80,$80,$00,$00,$e4
	.db $e5,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e7

timeOutTable
	.db $e0,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e2
	.db $e3,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$e4
	.db $e3,$00,$00,$87,$86,$88,$82,$80,$86,$e9,$80,$84,$eb,$87,$f0,$00,$00,$e4
	.db $e3,$00,$00,$80,$80,$80,$80,$80,$80,$80,$80,$80,$80,$80,$80,$00,$00,$e4
	.db $e3,$00,$00,$80,$00,$00,$80,$80,$80,$80,$80,$00,$00,$00,$80,$00,$00,$e4
	.db $e3,$00,$00,$00,$f2,$ea,$88,$82,$00,$80,$84,$83,$82,$e8,$00,$00,$00,$e4
	.db $e3,$00,$00,$80,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$e4
	.db $e5,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e7

version
	.db "Lan Master v1.09 14.06.11"

    .org $fffa
    .dw  NMI_CALL
    .dw  reset
	.dw  0


    .bank 4
    .org $0000
    .incbin "patterns.chr"