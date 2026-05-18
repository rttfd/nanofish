#!/usr/bin/env bash
set -euo pipefail

readonly FEATURE_SETS=(
  "tokio_impl"
  "tokio_impl,ws"
  "tokio_impl,log"
  "tokio_impl,ws,log"
  "embassy_impl"
  "embassy_impl,proto-ipv6"
  "embassy_impl,ws"
  "embassy_impl,ws,proto-ipv6"
  "embassy_impl,defmt"
  "embassy_impl,defmt,proto-ipv6"
  "embassy_impl,ws,defmt"
  "embassy_impl,ws,defmt,proto-ipv6"
)

simple_run(){
  local work_dir="$1"
  local cargo_subcommand="$2"
  shift 2

  local -a extra_args=("$@")

  if ((${#extra_args[@]} == 0)); then
    echo "Command: cd $work_dir && cargo $cargo_subcommand"
    (
      cd "$work_dir"
      cargo "$cargo_subcommand"
    )
  else
    echo "Command: cd $work_dir && cargo $cargo_subcommand ${extra_args[*]}"
    (
      cd "$work_dir"
      cargo "$cargo_subcommand" "${extra_args[@]}"
    )
  fi
}

run_for_all_features() {
  local work_dir="$1"
  local cargo_subcommand="$2"
  shift 2

  local -a extra_args=("$@")

  for feature_set in "${FEATURE_SETS[@]}"; do
    printf '\n==> %s [%s]\n' "$cargo_subcommand" "$feature_set"
    if ((${#extra_args[@]} == 0)); then
      # Trace command line for better visibility when running with `all`.
      echo "Command: cd $work_dir && cargo $cargo_subcommand --no-default-features --features $feature_set"
      # Run inside a subshell so cd never affects the parent shell's $PWD,
      # even if the cargo command fails.
      (
        cd "$work_dir"
        cargo "$cargo_subcommand" --no-default-features --features "$feature_set"
      )
    else
      # Trace command line for better visibility when running with `all`.
      echo "Command: cd $work_dir && cargo $cargo_subcommand --no-default-features --features $feature_set ${extra_args[*]}"
      (
        cd "$work_dir"
        cargo "$cargo_subcommand" --no-default-features --features "$feature_set" "${extra_args[@]}"
      )
    fi
  done
}

case "${1:-all}" in
  list)
    printf '%s\n' "${FEATURE_SETS[@]}"
    ;;
  build)
    run_for_all_features . build
    simple_run ./demos/rasberry_pico_w build
    simple_run ./demos/tokio_hello_world build
    ;;
  clippy)
    run_for_all_features . clippy --no-deps
    simple_run ./demos/rasberry_pico_w clippy --no-deps
    simple_run ./demos/tokio_hello_world clippy --no-deps
    ;;
  test)
    run_for_all_features . test
    ;;
  all)
    run_for_all_features . build
    simple_run ./demos/rasberry_pico_w build
    simple_run ./demos/tokio_hello_world build
    run_for_all_features . clippy --no-deps
    simple_run ./demos/rasberry_pico_w clippy --no-deps
    simple_run ./demos/tokio_hello_world clippy --no-deps
    run_for_all_features . test
    ;;
  *)
    echo "usage: $0 [list|build|clippy|test|all]" >&2
    exit 1
    ;;
esac
