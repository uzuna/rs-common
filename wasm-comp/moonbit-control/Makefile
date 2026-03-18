REPO_ROOT := $(shell git rev-parse --show-toplevel)
WIT_DIR   := $(REPO_ROOT)/examples/moonbit-runner/wit
PLUGINS_DIR := $(REPO_ROOT)/examples/moonbit-runner/plugins
WASM_OUT  := $(PLUGINS_DIR)/control.component.wasm
BUILD_WASM := _build/wasm/release/build/gen/gen.wasm

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

$(WASM_OUT): $(BUILD_WASM) | $(PLUGINS_DIR)
	wasm-tools component embed $(WIT_DIR) $(BUILD_WASM) --encoding utf16 --output control.wasm
	wasm-tools component new control.wasm --output $@
	rm -f control.wasm

$(PLUGINS_DIR):
	mkdir -p $@

.PHONY: build
build: $(WASM_OUT)

.PHONY: clean
clean:
	moon clean
	rm -f control.wasm $(WASM_OUT)
