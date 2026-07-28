.PHONY: build test fmt lint check bench deploy e2e clean optimize

# Default target
all: build

build:
	cargo build --release --target wasm32v1-none

optimize: build
	stellar contract optimize --wasm target/wasm32v1-none/release/amm.wasm
	stellar contract optimize --wasm target/wasm32v1-none/release/token.wasm
	stellar contract optimize --wasm target/wasm32v1-none/release/factory.wasm
	stellar contract optimize --wasm target/wasm32v1-none/release/governance.wasm
	stellar contract optimize --wasm target/wasm32v1-none/release/twap_consumer.wasm

test: build
	cargo test

fmt:
	cargo fmt --all

lint:
	cargo clippy --all -- -D warnings

check: fmt lint test

bench:
	cargo run -p benches -- --check

deploy:
	bash scripts/deploy.sh

e2e:
	bash scripts/e2e.sh

clean:
	cargo clean
