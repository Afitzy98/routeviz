.PHONY: help bench bench-quick wasm web dev build test lint clean

BENCH_OUT ?= web/public/benchmarks.json

help:
	@echo "Usage: make <target>"
	@echo ""
	@echo "  bench        Run full benchmarks and write results to $(BENCH_OUT)"
	@echo "  bench-quick  Run benchmarks with fewer samples (faster)"
	@echo "  wasm         Build the WASM package (wasm-pack)"
	@echo "  web          Install web dependencies"
	@echo "  dev          Start the Vite dev server"
	@echo "  build        Build the web app (wasm + npm run build)"
	@echo "  test         Run Rust tests"
	@echo "  lint         Run clippy + eslint"
	@echo "  clean        Remove build artifacts"

bench:
	cargo run --release --bin bench -- --out $(BENCH_OUT)

bench-quick:
	cargo run --release --bin bench -- --samples 100 --out $(BENCH_OUT)

wasm:
	wasm-pack build wasm --target web --out-dir pkg

web:
	cd web && npm install

dev: wasm
	cd web && npm run dev

build: wasm
	cd web && npm run build

test:
	cargo test

lint:
	cargo clippy -- -D warnings
	cd web && npm run lint

clean:
	cargo clean
	rm -rf wasm/pkg web/dist
