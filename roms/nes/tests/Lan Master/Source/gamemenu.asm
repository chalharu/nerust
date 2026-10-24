gameMenu
	lda #SFX_SELECT
	jsr sfxPlay

	lda #0
	sta <MENU_CUR
	sta <MENU_CNT

	jsr waitNMI

	lda #$69
	sta <TEMP
	lda #$21
	sta <TEMP+1

	ldx #0

.read1
	lda <TEMP+1
	sta PPU_ADDR
	lda <TEMP
	sta PPU_ADDR
	lda PPU_DATA

	lda <TEMP
	clc
	adc #32
	sta <TEMP
	lda <TEMP+1
	adc #0
	sta <TEMP+1

	lda PPU_DATA
	sta GAME_MENU_BUF,x
	lda PPU_DATA
	sta GAME_MENU_BUF+1,x
	lda PPU_DATA
	sta GAME_MENU_BUF+2,x
	lda PPU_DATA
	sta GAME_MENU_BUF+3,x
	lda PPU_DATA
	sta GAME_MENU_BUF+4,x
	lda PPU_DATA
	sta GAME_MENU_BUF+5,x
	lda PPU_DATA
	sta GAME_MENU_BUF+6,x
	lda PPU_DATA
	sta GAME_MENU_BUF+7,x
	lda PPU_DATA
	sta GAME_MENU_BUF+8,x
	lda PPU_DATA
	sta GAME_MENU_BUF+9,x
	lda PPU_DATA
	sta GAME_MENU_BUF+10,x
	lda PPU_DATA
	sta GAME_MENU_BUF+11,x
	lda PPU_DATA
	sta GAME_MENU_BUF+12,x
	lda PPU_DATA
	sta GAME_MENU_BUF+13,x

	txa
	clc
	adc #14
	tax

	cpx #140
	bne .read1

	jsr resetPPUAdr
	jsr FamiToneUpdate

	jsr waitNMI
	jsr updateStats
	lda #%00001110	;enable display, disable sprites
	sta PPU_MASK

	lda #$69
	sta <TEMP
	lda #$21
	sta <TEMP+1

	ldx #0

.show1
	lda <TEMP+1
	sta PPU_ADDR
	lda <TEMP
	sta PPU_ADDR

	lda <TEMP
	clc
	adc #32
	sta <TEMP
	lda <TEMP+1
	adc #0
	sta <TEMP+1

	lda gameMenuTable,x
	sta PPU_DATA
	lda gameMenuTable+1,x
	sta PPU_DATA
	lda gameMenuTable+2,x
	sta PPU_DATA
	lda gameMenuTable+3,x
	sta PPU_DATA
	lda gameMenuTable+4,x
	sta PPU_DATA
	lda gameMenuTable+5,x
	sta PPU_DATA
	lda gameMenuTable+6,x
	sta PPU_DATA
	lda gameMenuTable+7,x
	sta PPU_DATA
	lda gameMenuTable+8,x
	sta PPU_DATA
	lda gameMenuTable+9,x
	sta PPU_DATA
	lda gameMenuTable+10,x
	sta PPU_DATA
	lda gameMenuTable+11,x
	sta PPU_DATA
	lda gameMenuTable+12,x
	sta PPU_DATA
	lda gameMenuTable+13,x
	sta PPU_DATA

	txa
	clc
	adc #14
	tax

	cpx #140
	bne .show1

	lda <MENU_CUR
	jsr gameMenuShowCurAttr

	lda #$22
	sta PPU_ADDR
	lda #$71
	sta PPU_ADDR

	jsr showPassCode
	jsr resetPPUAdr
	jsr FamiToneUpdate

gameMenuLoop
	jsr waitNMI50
	jsr gameMenuShowCur
	lda <MENU_CUR
	jsr gameMenuShowCurAttr
	jsr FamiToneUpdate

	inc <MENU_CNT

	jsr padPoll

	lda <PAD_STATET
	and #PAD_UP
	beq .1
	lda #SFX_CURSOR
	jsr sfxPlay
	lda #0
	sta <MENU_CNT
	dec <MENU_CUR
	bpl .1
	lda #2
	sta <MENU_CUR
	jmp gameMenuLoop
.1
	lda <PAD_STATET
	and #PAD_DOWN
	beq .2
	lda #SFX_CURSOR
	jsr sfxPlay
	lda #0
	sta <MENU_CNT
	inc <MENU_CUR
	lda <MENU_CUR
	cmp #3
	bne .2
	lda #0
	sta <MENU_CUR
	jmp gameMenuLoop
.2
	lda <PAD_STATET
	and #(PAD_A|PAD_B|PAD_START)
	beq .3
	jmp gameMenuDone
.3
	jmp gameMenuLoop

gameMenuDone
	lda #SFX_START
	jsr sfxPlay

	jsr waitNMI
	lda #0
	sta <MENU_CNT
	jsr gameMenuShowCur
	jsr FamiToneUpdate

	lda #4
	sta <TEMP
.hide1
	jsr waitNMI50
	jsr FamiToneUpdate
	jsr waitNMI50
	lda <MENU_CUR
	jsr gameMenuShowCurAttr
	jsr FamiToneUpdate
	jsr waitNMI50
	jsr FamiToneUpdate
	jsr waitNMI50
	lda #3
	jsr gameMenuShowCurAttr
	jsr FamiToneUpdate
	dec <TEMP
	bne .hide1

	lda <MENU_CUR
	cmp #0
	beq .resume

	lda <MENU_CUR
	cmp #1
	beq .restart

	jmp exitGame

.restart
	jsr gameMenuClose
	jmp restartLevel

.resume
	jsr gameMenuClose
	jmp gameLoopInit


gameMenuClose
	jsr waitNMI
	lda #%00011110	;enable display, enable sprites
	sta PPU_MASK

	lda #$69
	sta <TEMP
	lda #$21
	sta <TEMP+1

	ldx #0
.hide2
	lda <TEMP+1
	sta PPU_ADDR
	lda <TEMP
	sta PPU_ADDR

	lda <TEMP
	clc
	adc #32
	sta <TEMP
	lda <TEMP+1
	adc #0
	sta <TEMP+1

	lda GAME_MENU_BUF,x
	sta PPU_DATA
	lda GAME_MENU_BUF+1,x
	sta PPU_DATA
	lda GAME_MENU_BUF+2,x
	sta PPU_DATA
	lda GAME_MENU_BUF+3,x
	sta PPU_DATA
	lda GAME_MENU_BUF+4,x
	sta PPU_DATA
	lda GAME_MENU_BUF+5,x
	sta PPU_DATA
	lda GAME_MENU_BUF+6,x
	sta PPU_DATA
	lda GAME_MENU_BUF+7,x
	sta PPU_DATA
	lda GAME_MENU_BUF+8,x
	sta PPU_DATA
	lda GAME_MENU_BUF+9,x
	sta PPU_DATA
	lda GAME_MENU_BUF+10,x
	sta PPU_DATA
	lda GAME_MENU_BUF+11,x
	sta PPU_DATA
	lda GAME_MENU_BUF+12,x
	sta PPU_DATA
	lda GAME_MENU_BUF+13,x
	sta PPU_DATA

	txa
	clc
	adc #14
	tax

	cpx #140
	bne .hide2

	lda #4
	jsr gameMenuShowCurAttr

	jsr FamiToneUpdate

	rts

gameMenuShowCur
	ldx #0
	lda #$8a
	sta TEMP
	lda #$21
	sta TEMP+1
.showcur1
	lda TEMP+1
	sta PPU_ADDR
	lda TEMP
	sta PPU_ADDR
	ldy #0
	cpx <MENU_CUR
	bne .showcur2
	lda <MENU_CNT
	and #16
	bne .showcur2
	ldy #$47
.showcur2
	sty PPU_DATA
	lda TEMP
	clc
	adc #64
	sta TEMP
	lda TEMP+1
	adc #0
	sta TEMP+1
	inx
	cpx #3
	bne .showcur1

	jmp resetPPUAdr


gameMenuShowCurAttr
	asl a
	asl a
	asl a
	tax

	lda #$23
	sta PPU_ADDR
	lda #$da
	sta PPU_ADDR
	lda gameMenuAttrTable,x
	sta PPU_DATA
	lda gameMenuAttrTable+1,x
	sta PPU_DATA
	lda gameMenuAttrTable+2,x
	sta PPU_DATA
	lda gameMenuAttrTable+3,x
	sta PPU_DATA

	lda #$23
	sta PPU_ADDR
	lda #$e2
	sta PPU_ADDR
	lda gameMenuAttrTable+4,x
	sta PPU_DATA
	lda gameMenuAttrTable+5,x
	sta PPU_DATA
	lda gameMenuAttrTable+6,x
	sta PPU_DATA
	lda gameMenuAttrTable+7,x
	sta PPU_DATA

	jmp resetPPUAdr


gameMenuAttrTable
	.db $cc,$f0,$f0,$30,$0c,$0f,$0f,$03
	.db $cc,$0f,$0f,$03,$0c,$0f,$0f,$03
	.db $cc,$ff,$ff,$33,$0c,$00,$00,$00
	.db $cc,$ff,$ff,$33,$0c,$0f,$0f,$03
	.db $00,$00,$00,$00,$00,$00,$00,$00

gameMenuTable
	.db $e0,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e1,$e2
	.db $e3,$00,$00,$e8,$82,$e9,$eb,$88,$82,$00,$00,$00,$00,$e4
	.db $e3,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$e4
	.db $e3,$00,$00,$e8,$82,$e9,$87,$ea,$e8,$87,$00,$00,$00,$e4
	.db $e3,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$e4
	.db $e3,$00,$00,$88,$ea,$86,$85,$00,$88,$82,$85,$eb,$00,$e4
	.db $e3,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$e4
	.db $e3,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$e4
	.db $e3,$00,$ec,$84,$ed,$82,$00,$00,$ee,$ee,$ee,$ee,$00,$e4
	.db $e5,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e6,$e7
