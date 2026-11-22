WASM_TARGET := wasm32-unknown-unknown
WASM_OUT    := target/$(WASM_TARGET)/release/defender.wasm
WEB_DIR     := web
PORT        ?= 8080

.PHONY: all build test run clean

all: build

build:
	cargo build --target $(WASM_TARGET) --release
	cp $(WASM_OUT) $(WEB_DIR)/defender.wasm

test:
	cargo test

run: build
	@echo "Serving Defender at http://localhost:$(PORT)"
	python3 -m http.server $(PORT) --directory $(WEB_DIR)

clean:
	cargo clean
	rm -f $(WEB_DIR)/defender.wasm
