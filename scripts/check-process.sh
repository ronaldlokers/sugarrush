#!/usr/bin/env bash
# Enforce the repo conventions that live in CLAUDE.md prose, so they fail a PR
# rather than a review. Everything checkable from the source itself is a Rust
# test (see src/app.rs); this covers what needs to read the files as text.
#
# Usage: scripts/check-process.sh
set -euo pipefail

fail=0
err() {
	echo "::error::$*"
	fail=1
}

# 1. Every `Field` variant is in `Field::ALL`. A variant left out compiles
#    fine — `label`/`group` are exhaustive matches — but the setting simply
#    never appears on the settings screen, which is the failure this catches.
variants=$(sed -n '/^pub enum Field {$/,/^}$/p' src/app.rs |
	sed -n 's/^    \([A-Z][A-Za-z0-9]*\),$/\1/p')
listed=$(sed -n '/pub const ALL: \[Field; [0-9]*\] = \[/,/^    \];$/p' src/app.rs |
	sed -n 's/^        Field::\([A-Za-z0-9]*\),$/\1/p')
for v in $variants; do
	if ! grep -qx "$v" <<<"$listed"; then
		err "Field::$v is missing from Field::ALL — the setting would never render."
	fi
done
declared=$(sed -n 's/.*pub const ALL: \[Field; \([0-9]*\)\].*/\1/p' src/app.rs)
count=$(wc -l <<<"$listed")
if [ "$declared" != "$count" ]; then
	err "Field::ALL declares $declared entries but lists $count."
fi

# 2. Config-key coverage used to be checked here with `sed`, which only saw
#    fields written as `key: Some(...)` — so `units`, `refresh_secs`,
#    `graph_style`, `agp_days`, `sites` and the whole `minimap` table were
#    never checked, and the gate reported success while covering about half of
#    what it claimed. It is now a Rust test that serializes a Config and
#    asserts every emitted key appears in config.example.toml, which is
#    exhaustive by construction: see `every_persisted_key_is_documented`.

if [ "$fail" -eq 0 ]; then
	echo "process checks passed"
fi
exit "$fail"
