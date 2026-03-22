REPO_ROOT := $(shell git rev-parse --show-toplevel)
WIT_DIR   := $(REPO_ROOT)/examples/moonbit-runner/wit
PLUGINS_DIR := $(REPO_ROOT)/examples/moonbit-runner/plugins
WASM_OUT  := $(PLUGINS_DIR)/control.component.wasm
RAW_WASM_OUT := $(PLUGINS_DIR)/control.core.wasm
RAW_WASM_SYMBOLIZED_OUT := $(PLUGINS_DIR)/control.core.symbolized.wasm
BUILD_WASM := _build/wasm/release/build/gen/gen.wasm
DEBUG_BUILD_WASM := _build/wasm/debug/build/gen/gen.wasm

.PHONY: all
all: $(WASM_OUT)

# Generate MoonBit project from WIT (re-run when WIT changes)
.PHONY: gen
gen:
	wit-bindgen moonbit $(WIT_DIR)/world.wit --out-dir . --derive-eq --derive-show

.PHONY: check
check:
	moon check --target wasm

$(BUILD_WASM): $(shell find gen world -name "*.mbt" 2>/dev/null)
	moon build --target wasm --release

$(DEBUG_BUILD_WASM): $(shell find gen world -name "*.mbt" 2>/dev/null)
	moon build --target wasm

$(WASM_OUT): $(BUILD_WASM) | $(PLUGINS_DIR)
	wasm-tools component embed $(WIT_DIR) $(BUILD_WASM) --encoding utf16 --output control.wasm
	wasm-tools component new control.wasm --output $@
	rm -f control.wasm

$(PLUGINS_DIR):
	mkdir -p $@

.PHONY: build
build: $(WASM_OUT)

.PHONY: build-raw
build-raw: $(BUILD_WASM) | $(PLUGINS_DIR)
	cp $(BUILD_WASM) $(RAW_WASM_OUT)

.PHONY: build-raw-symbolized
build-raw-symbolized: $(DEBUG_BUILD_WASM) | $(PLUGINS_DIR)
	cp $(DEBUG_BUILD_WASM) $(RAW_WASM_SYMBOLIZED_OUT)

.PHONY: build-all
build-all: $(WASM_OUT) $(RAW_WASM_OUT)

.PHONY: clean
clean:
	moon clean
	rm -f control.wasm $(WASM_OUT) $(RAW_WASM_OUT) $(RAW_WASM_SYMBOLIZED_OUT)
