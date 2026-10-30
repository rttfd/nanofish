# AGENTS.md

Rules for anyone changing this crate. `CONTRIBUTING.md` covers the git workflow; this file covers
the code. They are the same rules I use for Rust apps: a library is somebody else's app core, so
nothing is waived because it's a library.

## Commands

Every task is a make target, locally and in CI. Never call `cargo` directly in docs or CI: add or
change a target instead. `make` alone lists them.

- `make ci`: everything CI runs (`fmt-check`, `check`, `lint`, `test-all`, `doc-check`). Run it
  before every push.
- `make lint`: clippy, pedantic, on all targets and every feature combination, warnings as errors.
- `make bench` / `make bench-save` / `make bench-compare` (`BASELINE=main`, `BENCH=<filter>`).

`defmt` and `log` can't be enabled together (Embassy refuses it), so `--all-features` never
builds; the make targets list the combinations that matter.

## Architecture: ports and adapters

- **Core**: transport-free HTTP. `HttpClient`, `HttpServer` and the codec work over any
  `embedded-io-async` stream and know nothing about Embassy, smoltcp, or TLS.
- **Adapters** sit behind features and are whole modules: `client/embassy/`, `server/embassy.rs`,
  `client/tls.rs`, `smoltcp.rs`, and the error conversions in `error/embassy.rs` and
  `error/tls.rs`.
- **The caller is the composition root.** The crate exports the core and the adapters; the caller
  picks and wires them. Convenience types that pick adapters (`EmbassyHttpClient`) live in their
  adapter module, never in the core.

## Modules and features

- Declare every module in `src/lib.rs` or its folder's `mod.rs`. A module with parts is a folder
  with a `mod.rs`, never `name.rs` next to `name/`.
- Put `#[cfg(feature = "...")]` on **module declarations**, not on items inside a file. Each
  variant is its own file, written as if always compiled (see `client/embassy/tls.rs` and
  `client/embassy/no_tls.rs`). Accepted exceptions: a single enum variant or match arm, a
  `derive` or trait impl behind `defmt`, and the `fmt.rs` logging shim.
- Refer to the crate's own items as `nanofish::...` (`extern crate self as nanofish;` in
  `lib.rs`), never `crate::` or `super::`, in tests and doc links too. Only visibility keeps
  `pub(crate)` and `pub(super)`.

## Errors and lints

- No `unwrap`, `expect` or `panic` outside tests: `deny` in `Cargo.toml`, opted out for tests in
  `.clippy.toml`, and with `#![expect(...)]` at the top of a benchmark. Bad input or a failed
  read is an `Error`, never a panic on the caller's behalf.
- Fix lints instead of silencing them. If an exception is truly needed, use
  `#[expect(lint, reason = "...")]` with a real reason, never `#[allow]`.
- Public items document `# Errors` on anything returning `Result`, and carry `#[must_use]` where
  clippy asks for it.

## Tests and benchmarks

- Test the core through its ports with fakes (in-memory streams), not a real network. Name tests
  after the guarantee they check, including the unhappy paths.
- Benchmarks in `benches/` cover the per-request hot paths through the public API. Update one when
  you change a hot path, and compare against a saved baseline (`make bench-save` on `main`, then
  `make bench-compare` on your branch) instead of trusting a single run. The `profiling` profile
  is optimized with symbols, for `samply` and `cargo flamegraph`.

## Pinned dependencies

- `rand_core` stays on 0.6: `embedded-tls` 0.19 exposes `rand_core` 0.6 traits in its provider
  API, so 0.9 is trait-incompatible. Don't bump it until `embedded-tls` does.
