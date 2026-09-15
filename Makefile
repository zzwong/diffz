.DEFAULT_GOAL := help

KACHE_VERSION := 0.21.0
KACHE_ROOT := target/kache/$(KACHE_VERSION)
KACHE_BIN := $(KACHE_ROOT)/bin/kache
KACHE_CONFIG := $(abspath scripts/kache.toml)
KACHE_SOCKET := $(abspath target/kache/runtime/daemon.sock)
KACHE_ENV := KACHE_CONFIG="$(KACHE_CONFIG)" KACHE_HOST_CONFIG= KACHE_SOCKET_PATH="$(KACHE_SOCKET)" RUSTC_WRAPPER="$(abspath $(KACHE_BIN))"

.PHONY: help kache build build-release build-plain check check-native package-macos

help:
	@printf '%s\n' \
		'make build         Build diffz with local Kache' \
		'make build-release Build diffz with local Kache in release mode' \
		'make build-plain   Build diffz with regular Cargo' \
		'make check         Run core checks with local Kache' \
		'make check-native  Run native checks with local Kache' \
		'make package-macos Package the macOS DMG with local Kache' \
		'make help          List development commands'

kache:
	@if [ ! -x "$(KACHE_BIN)" ]; then \
		mkdir -p "$(KACHE_ROOT)"; \
		RUSTC_WRAPPER= RUSTC_WORKSPACE_WRAPPER= cargo install --locked --force \
			--version "$(KACHE_VERSION)" --root "$(KACHE_ROOT)" kache; \
	fi
	@test "$$($(KACHE_BIN) --version)" = "kache $(KACHE_VERSION)" || \
		{ printf 'expected kache %s at %s\n' "$(KACHE_VERSION)" "$(KACHE_BIN)" >&2; exit 1; }

build: kache
	$(KACHE_ENV) cargo build --locked -p diffz

build-release: kache
	$(KACHE_ENV) cargo build --locked --release -p diffz

build-plain:
	RUSTC_WRAPPER= RUSTC_WORKSPACE_WRAPPER= cargo build --locked -p diffz

check: kache
	$(KACHE_ENV) bash scripts/check.sh core

check-native: kache
	$(KACHE_ENV) bash scripts/check.sh native

package-macos: kache
	$(KACHE_ENV) bash scripts/package-macos.sh
