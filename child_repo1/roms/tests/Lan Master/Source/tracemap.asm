T_CURLIST		equ $80	;0 or 128, offset in lists page (max length is 128)
T_NEWLIST		equ $81 ;0 or 128, offset in lists page
T_CURLEN		equ $82
T_MAPOFF		equ $83
T_DIR			equ $84
T_LISTPTR		equ $85
T_SHIFT			equ $86
T_ONLINE		equ $87 ;word
T_ONLINE_CNT	equ $89

T_LISTPAGE		equ $600

T_LEFT			equ 1
T_RIGHT			equ 2
T_UP			equ 4
T_DOWN			equ 8


traceMap
	ldx #0				;clear check map
	txa
.1
	sta GAME_CHECK,x
	inx
	bne .1

	stx <T_CURLIST			;init variables
	lda #$80
	sta <T_NEWLIST
	stx <T_ONLINE
	stx <T_ONLINE+1
	stx <T_ONLINE_CNT
	lda #1
	sta <T_CURLEN

	ldx <GAME_TERM_TRACE
	bne .2
	lda #0
	sta <GAME_ONLINE
	sta <GAME_ONLINE+1
.2
	lda GAME_TERM_LIST,x
	sta T_LISTPAGE
	inc <GAME_TERM_TRACE
	lda <GAME_TERM_TRACE
	cmp <GAME_TERM_CNT
	bne .3
	lda #0
	sta <GAME_TERM_TRACE
.3

.traceMainLoop
	lda #0
	sta <T_SHIFT
	lda <T_CURLIST
	sta <T_LISTPTR

.traceInnerLoop
	ldx <T_LISTPTR
	lda T_LISTPAGE,x
	cmp #$ff
	bne .noSkip
	sta <T_SHIFT
	jmp .noDown
.noSkip
	sta <T_MAPOFF
	tax
	lda GAME_MAP,x
	tax
	lda pinsTable,x
	ldy <T_SHIFT
	beq .5
	ror a
	ror a
	ror a
	ror a
	and #$0f
.5
	sta <T_DIR
	lda #0
	sta <T_SHIFT

	ldx <T_MAPOFF
	lda GAME_CHECK,x
	beq .6
	lda GAME_MAP,x
	cmp #15
	bcc .6
	lda <T_ONLINE			;count found terminals
	clc
	adc <GAME_TERM_FP
	sta <T_ONLINE
	lda <T_ONLINE+1
	adc <GAME_TERM_FP+1
	sta <T_ONLINE+1
	inc <T_ONLINE_CNT
.6

	lda <T_DIR
	and #T_LEFT
	beq .noLeft

	ldy <T_MAPOFF
	dey
	lda GAME_CHECK,y
	and #1
	bne .left0

	lda GAME_MAP,y
	tax
	lda pinsTable,x
	and #T_RIGHT
	beq .left0

	ldx <T_NEWLIST
	tya
	sta T_LISTPAGE,x
	inc <T_NEWLIST
	tax
	lda GAME_CHECK,x
	ora #1
	sta GAME_CHECK,x

.left0
	lda GAME_CHECK,y
	and #2
	bne .noLeft

	lda GAME_MAP,y
	tax
	lda pinsTable,x
	and #T_RIGHT<<4
	beq .noLeft

	ldx <T_NEWLIST
	lda #$ff
	sta T_LISTPAGE,x
	inx
	tya
	sta T_LISTPAGE,x
	inx
	stx <T_NEWLIST
	tax
	lda GAME_CHECK,x
	ora #2
	sta GAME_CHECK,x

.noLeft
	lda <T_DIR
	and #T_RIGHT
	beq .noRight

	ldy <T_MAPOFF
	iny
	lda GAME_CHECK,y
	and #1
	bne .right0

	lda GAME_MAP,y
	tax
	lda pinsTable,x
	and #T_LEFT
	beq .right0

	ldx <T_NEWLIST
	tya
	sta T_LISTPAGE,x
	inc <T_NEWLIST
	tax
	lda GAME_CHECK,x
	ora #1
	sta GAME_CHECK,x

.right0
	lda GAME_CHECK,y
	and #2
	bne .noRight

	lda GAME_MAP,y
	tax
	lda pinsTable,x
	and #T_LEFT<<4
	beq .noRight

	ldx <T_NEWLIST
	lda #$ff
	sta T_LISTPAGE,x
	inx
	tya
	sta T_LISTPAGE,x
	inx
	stx <T_NEWLIST
	tax
	lda GAME_CHECK,x
	ora #2
	sta GAME_CHECK,x

.noRight
	lda <T_DIR
	and #T_UP
	beq .noUp

	lda <T_MAPOFF
	clc
	adc #-16
	tay
	lda GAME_CHECK,y
	and #1
	bne .up0

	lda GAME_MAP,y
	tax
	lda pinsTable,x
	and #T_DOWN
	beq .up0

	ldx <T_NEWLIST
	tya
	sta T_LISTPAGE,x
	inc <T_NEWLIST
	tax
	lda GAME_CHECK,x
	ora #1
	sta GAME_CHECK,x

.up0
	lda GAME_CHECK,y
	and #2
	bne .noUp

	lda GAME_MAP,y
	tax
	lda pinsTable,x
	and #T_DOWN<<4
	beq .noUp

	ldx <T_NEWLIST
	lda #$ff
	sta T_LISTPAGE,x
	inx
	tya
	sta T_LISTPAGE,x
	inx
	stx <T_NEWLIST
	tax
	lda GAME_CHECK,x
	ora #2
	sta GAME_CHECK,x

.noUp
	lda <T_DIR
	and #T_DOWN
	beq .noDown

	lda <T_MAPOFF
	clc
	adc #16
	tay
	lda GAME_CHECK,y
	and #1
	bne .down0

	lda GAME_MAP,y
	tax
	lda pinsTable,x
	and #T_UP
	beq .down0

	ldx <T_NEWLIST
	tya
	sta T_LISTPAGE,x
	inc <T_NEWLIST
	tax
	lda GAME_CHECK,x
	ora #1
	sta GAME_CHECK,x

.down0
	lda GAME_CHECK,y
	and #2
	bne .noDown

	lda GAME_MAP,y
	tax
	lda pinsTable,x
	and #T_UP<<4
	beq .noDown

	ldx <T_NEWLIST
	lda #$ff
	sta T_LISTPAGE,x
	inx
	tya
	sta T_LISTPAGE,x
	inx
	stx <T_NEWLIST
	tax
	lda GAME_CHECK,x
	ora #2
	sta GAME_CHECK,x

.noDown
	inc <T_LISTPTR
	dec <T_CURLEN
	beq .innerLoopDone
	jmp .traceInnerLoop
.innerLoopDone

	lda <T_NEWLIST
	and #$7f
	bne .loop
	
	lda <T_ONLINE+1
	cmp <GAME_ONLINE+1
	bcc .done
	lda <T_ONLINE
	cmp <GAME_ONLINE
	bcc .done
	lda <T_ONLINE
	sta <GAME_ONLINE
	lda <T_ONLINE+1
	sta <GAME_ONLINE+1
.done
	lda <GAME_TERM_CNT
	cmp <T_ONLINE_CNT
	rts
	
.loop

	sta <T_CURLEN
	lda <T_CURLIST
	and #$80
	sta <T_NEWLIST
	eor #$80
	sta <T_CURLIST

	jmp .traceMainLoop


pinsTable
	.db 0
	.db T_LEFT|T_RIGHT
	.db T_UP|T_DOWN
	.db T_LEFT|T_RIGHT|((T_UP|T_DOWN)<<4)
	.db T_LEFT|T_RIGHT|T_UP|T_DOWN
	.db T_LEFT|T_DOWN
	.db T_LEFT|T_UP
	.db T_UP|T_RIGHT
	.db T_DOWN|T_RIGHT
	.db T_LEFT|T_RIGHT|T_DOWN
	.db T_LEFT|T_UP|T_DOWN
	.db T_LEFT|T_UP|T_RIGHT
	.db T_UP|T_RIGHT|T_DOWN
	.db T_LEFT|T_UP|((T_DOWN|T_RIGHT)<<4)
	.db T_LEFT|T_DOWN|((T_UP|T_RIGHT)<<4)
	.db T_RIGHT
	.db T_DOWN
	.db T_LEFT
	.db T_UP