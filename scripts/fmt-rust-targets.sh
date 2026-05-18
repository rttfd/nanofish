set -euo pipefail

(cd lib && cargo fmt --all -- --config edition=2024)

for example_dir in demos/*/; do
	[[ -d "$example_dir" ]] || continue

	if [[ -f "${example_dir}Cargo.toml" || -f "${example_dir}cargo.toml" ]]; then
		(cd "$example_dir" && cargo fmt --all -- --config edition=2024)
	fi
done
