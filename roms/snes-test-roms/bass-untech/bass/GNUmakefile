name := bass-untech
build := release
console := true
flags += -I..

nall.path := ../nall
include $(nall.path)/GNUmakefile

objects := obj/bass.o

ifeq ($(platform), windows)
	name := $(name).exe
endif

obj/bass.o: bass.cpp

all: out/$(name) out-architectures

out/$(name): out obj $(objects)
	$(info Linking out/$(name) ...)
	+@$(compiler) -o out/$(name) $(objects) $(options)

verbose: nall.verbose all;

clean:
	$(call delete,obj/*)
	$(call delete,out/$(name))
	$(call delete,out/architectures/*)

obj out:
	mkdir $@

install: all
ifeq ($(shell id -un),root)
	$(error "make install should not be run as root")
else ifneq ($(filter $(platform),linux bsd),)
	cp out/$(name) $(prefix)/bin/$(name)
	mkdir -p $(prefix)/share/$(name)/architectures/
	cp -R data/architectures/* $(prefix)/share/$(name)/architectures/
endif

uninstall:
ifeq ($(shell id -un),root)
	$(error "make uninstall should not be run as root")
else ifneq ($(filter $(platform),linux bsd),)
	rm -f $(prefix)/bin/$(name)
endif


# Copy architectures to out/ so bass-untech can be used without installing it
architectures := $(wildcard data/architectures/*.arch)

.PHONY: out-architectures
out-architectures: out/architectures
out-architectures: $(architectures:data/architectures/%=out/architectures/%)

out/architectures/%.arch: data/architectures/%.arch
ifeq ($(platform), windows)
	copy "$(subst /,\,$<)" "$(subst /,\,$@)"
else
	cp "$<" "$@"
endif

out/architectures:
ifeq ($(platform), windows)
	mkdir out\architectures
else
	mkdir -p out/architectures
endif

-include obj/*.d
