ASM     := ./tools/wla-dx/wla-65816
LINK    := ./tools/wla-dx/wlalink
BASS    := ./tools/bass/bass.exe
PYTHON  := python3

ROM     := build/HiRomGsuTest.sfc
MSU     := build/HiRomGsuTest.msu
OBJ     := build/test_rom.o
GSU_BIN    := build/pixel_test.bin
GSU_DEMO   := build/gsu_demo.bin
GSU_SCALER := build/sprite_scaler.bin
FONT       := build/font.bin

.PHONY: all clean

all: $(ROM) $(MSU)

build:
	mkdir -p build

$(FONT): gen_font.py | build
	$(PYTHON) gen_font.py $@

$(GSU_BIN): pixel_test.gsu | build
	$(BASS) -strict -o $@ $<

$(GSU_DEMO): gsu_demo.gsu | build
	$(BASS) -strict -o $@ $<

$(GSU_SCALER): sprite_scaler.gsu | build
	$(BASS) -strict -o $@ $<

$(OBJ): test_rom.65816 test_rom.h $(GSU_BIN) $(GSU_DEMO) $(GSU_SCALER) $(FONT) | build
	$(ASM) -o $< $@

$(ROM): $(OBJ) linkfile.lnk inject_signatures.py
	$(LINK) -dsr linkfile.lnk $@
	$(PYTHON) inject_signatures.py $@

$(MSU): | build
	$(PYTHON) -c "open('$@','wb').write(bytes(4096))"

clean:
	rm -rf build
