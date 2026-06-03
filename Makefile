# ==============================================================================
# Makefile for nanofish
# ==============================================================================
#
# This Makefile provides convenient commands for development, testing, and
# publishing nanofish. It uses the same commands as the GitHub Actions
# CI/CD workflows to ensure consistency between local development and CI.
#
# The Rust version is automatically extracted from Cargo.toml to ensure
# alignment with the project's rust-version requirement.
#
# Usage:
#   make help          - Show all available targets
#   make ci            - Run all CI checks (fmt-check, clippy-all, test-all)
#   make pre-commit    - Run pre-commit checks (fmt, clippy-all, test-all)
#   make publish       - Publish to crates.io
#
# ==============================================================================

.PHONY: help
help: ## Show this help message
	@echo "Available targets:"
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2}'

# Rust version - automatically extracted from Cargo.toml
RUST_VERSION := $(shell grep 'rust-version = ' Cargo.toml | head -1 | sed 's/.*rust-version = "\(.*\)"/\1/')

FEATURES ?=

.PHONY: install-hooks
install-hooks: ## Install git hooks (prevents direct push to main)
	git config core.hooksPath .githooks
	@echo "Git hooks installed from .githooks/"

.PHONY: install-rust
install-rust: ## Install Rust toolchain with required components
	rustup toolchain install $(RUST_VERSION)
	rustup component add rustfmt clippy --toolchain $(RUST_VERSION)

.PHONY: fmt
fmt: ## Format all code
	cargo +$(RUST_VERSION) fmt --all

.PHONY: fmt-check
fmt-check: ## Check code formatting
	cargo +$(RUST_VERSION) fmt --all --check

.PHONY: clippy
clippy: ## Run clippy lints
ifeq ($(FEATURES),)
	cargo +$(RUST_VERSION) clippy -- -D warnings -W clippy::pedantic
else
	cargo +$(RUST_VERSION) clippy --features "$(FEATURES)" -- -D warnings -W clippy::pedantic
endif

.PHONY: clippy-all
clippy-all: ## Run clippy on all feature combinations
	@for features in "" "tls" "log" "defmt" "tls,log" "tls,defmt"; do \
		echo "Running clippy with features: $$features"; \
		cargo +$(RUST_VERSION) clippy --features "$$features" -- -D warnings -W clippy::pedantic; \
	done

.PHONY: test
test: ## Run tests
ifeq ($(FEATURES),)
	cargo +$(RUST_VERSION) test
else
	cargo +$(RUST_VERSION) test --features "$(FEATURES)"
endif

.PHONY: test-all
test-all: ## Run tests on all feature combinations
	@for features in "" "tls" "log" "defmt" "tls,log" "tls,defmt"; do \
		echo "Running tests with features: $$features"; \
		cargo +$(RUST_VERSION) test --features "$$features"; \
	done

.PHONY: build
build: ## Build the crate
	cargo +$(RUST_VERSION) build

.PHONY: build-release
build-release: ## Build in release mode
	cargo +$(RUST_VERSION) build --release

.PHONY: doc
doc: ## Generate documentation
	cargo +$(RUST_VERSION) doc --no-deps

.PHONY: doc-open
doc-open: ## Generate and open documentation
	cargo +$(RUST_VERSION) doc --no-deps --open

.PHONY: clean
clean: ## Clean build artifacts
	cargo clean

.PHONY: ci
ci: fmt-check clippy-all test-all ## Run all CI checks

.PHONY: pre-commit
pre-commit: fmt clippy-all test-all ## Run pre-commit checks

.PHONY: verify-version
verify-version: ## Verify version configuration
	@echo "Checking crate configuration..."
	@VERSION=$$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'); \
	echo "  Version: $$VERSION"; \
	EDITION=$$(grep '^edition = ' Cargo.toml | head -1 | sed 's/edition = "\(.*\)"/\1/'); \
	echo "  Edition: $$EDITION"; \
	RUST_VER=$$(grep '^rust-version = ' Cargo.toml | head -1 | sed 's/rust-version = "\(.*\)"/\1/'); \
	echo "  Rust version: $$RUST_VER"

.PHONY: publish-dry-run
publish-dry-run: ## Dry run of publishing to crates.io
	cargo +$(RUST_VERSION) publish --dry-run

.PHONY: publish
publish: ## Publish to crates.io (requires CARGO_REGISTRY_TOKEN)
	cargo +$(RUST_VERSION) publish

.PHONY: release
release: ## Create and push a git tag for the current version
	@VERSION=$$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'); \
	echo "Creating tag v$$VERSION..."; \
	git tag "v$$VERSION"; \
	git push origin "v$$VERSION"

.PHONY: update-deps
update-deps: ## Update dependencies
	cargo update

.PHONY: all
all: ci doc ## Run all checks and build documentation