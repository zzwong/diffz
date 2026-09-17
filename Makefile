.DEFAULT_GOAL := help

KACHE_VERSION := 0.21.0
KACHE_ROOT := target/kache/$(KACHE_VERSION)
KACHE_BIN := $(KACHE_ROOT)/bin/kache
KACHE_CONFIG := $(abspath scripts/kache.toml)
KACHE_SOCKET := $(abspath target/kache/runtime/daemon.sock)
KACHE_ENV := KACHE_CONFIG="$(KACHE_CONFIG)" KACHE_HOST_CONFIG= KACHE_SOCKET_PATH="$(KACHE_SOCKET)" RUSTC_WRAPPER="$(abspath $(KACHE_BIN))"

# Development builds for `run` and `install-dev`. RELEASE=1 selects the release profile,
# KACHE=0 builds with regular Cargo, and BIN=FILE makes install-dev skip the build.
RELEASE ?= 0
KACHE ?= 1
ifeq ($(RELEASE),1)
DEV_PROFILE := --release
DEV_BIN := target/release/diffz
else
DEV_PROFILE :=
DEV_BIN := target/debug/diffz
endif
ifeq ($(KACHE),0)
DEV_KACHE :=
DEV_CARGO_ENV := RUSTC_WRAPPER= RUSTC_WORKSPACE_WRAPPER=
else
DEV_KACHE := kache
DEV_CARGO_ENV := $(KACHE_ENV)
endif
# Relative XDG paths are ignored, as diffz itself does.
DEV_STATE_DIR ?= $(or $(filter /%,$(XDG_STATE_HOME)),$(HOME)/.local/state)/diffz-dev
DEFAULT_PREFIX := $(HOME)/.local
PREFIX ?= $(DEFAULT_PREFIX)
# XDG_DATA_HOME applies only to the default PREFIX, so PREFIX=/tmp/x keeps everything under it.
ifeq ($(PREFIX),$(DEFAULT_PREFIX))
DATADIR ?= $(or $(filter /%,$(XDG_DATA_HOME)),$(PREFIX)/share)
else
DATADIR ?= $(PREFIX)/share
endif
DEV_INSTALL_ENV := PREFIX="$(PREFIX)" DATADIR="$(DATADIR)" DEV_STATE_DIR="$(DEV_STATE_DIR)"

.PHONY: help kache build build-release build-plain check check-native package-macos \
	dev-build run install-dev uninstall-dev

help:
	@printf '%s\n' \
		'make build         Build diffz with local Kache' \
		'make build-release Build diffz with local Kache in release mode' \
		'make build-plain   Build diffz with regular Cargo' \
		'make check         Run core checks with local Kache' \
		'make check-native  Run native checks with local Kache' \
		'make package-macos Package the macOS DMG with local Kache' \
		'make run ARGS=...  Build and run diffz with development state' \
		'make install-dev   Install diffz-dev and a "Diffz (dev)" launcher' \
		'make uninstall-dev Remove what install-dev installed' \
		'make help          List development commands' \
		'' \
		'run and install-dev take RELEASE=1, KACHE=0 (regular Cargo) and' \
		'DEV_STATE_DIR (default $$XDG_STATE_HOME/diffz-dev); install-dev and' \
		'uninstall-dev take absolute PREFIX (default ~/.local) and DATADIR paths;' \
		'install-dev takes BIN=FILE to install a binary that is already built.'

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

dev-build: $(DEV_KACHE)
	$(DEV_CARGO_ENV) cargo build --locked $(DEV_PROFILE) -p diffz

# ARGS is expanded by the shell, so quote arguments containing spaces inside it. A later
# --state-dir in ARGS overrides this one. `diffz --doctor` accepts no other argument.
run: dev-build
	$(DEV_BIN) $(if $(filter --doctor,$(ARGS)),,--state-dir "$(DEV_STATE_DIR)") $(ARGS)

install-dev: $(if $(BIN),,dev-build)
	$(DEV_INSTALL_ENV) DEV_BIN="$(or $(BIN),$(DEV_BIN))" bash scripts/install-dev.sh install

uninstall-dev:
	$(DEV_INSTALL_ENV) bash scripts/install-dev.sh uninstall
