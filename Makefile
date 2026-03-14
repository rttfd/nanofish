.PHONY: fmt fmt-check clippy clippy-all test test-all ci publish

FEATURES ?=

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

clippy:
ifeq ($(FEATURES),)
	cargo clippy -- -D warnings -W clippy::pedantic
else
	cargo clippy --features "$(FEATURES)" -- -D warnings -W clippy::pedantic
endif

clippy-all:
	@for features in "" "tls" "log" "defmt" "tls,log" "tls,defmt"; do \
		echo "Running clippy with features: $$features"; \
		cargo clippy --features "$$features" -- -D warnings -W clippy::pedantic; \
	done

test:
ifeq ($(FEATURES),)
	cargo test
else
	cargo test --features "$(FEATURES)"
endif

test-all:
	@for features in "" "tls" "log" "defmt" "tls,log" "tls,defmt"; do \
		echo "Running tests with features: $$features"; \
		cargo test --features "$$features"; \
	done

ci: fmt-check clippy-all test-all

publish:
	cargo publish