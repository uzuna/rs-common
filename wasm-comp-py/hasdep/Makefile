
.PHONY: bindings
bindings:
	uvx componentize-py -d wit -w hasdep bindings hasdep_guest

.PHONY: build
build:
	uvx componentize-py \
		--wit-path wit/world.wit \
		--world hasdep \
		componentize \
		--stub-wasi \
		app \
		-o hasdep-py.wasm
