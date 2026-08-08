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

# 2. Every settings field name that reaches config.toml is documented in
#    config.example.toml. Checked by key name, since that's what a user greps.
while read -r key; do
	[ -z "$key" ] && continue
	grep -q "$key" config.example.toml ||
		err "$key is persisted by build_config but absent from config.example.toml"
done <<<"$(sed -n '/fn build_config/,/^    }$/p' src/app.rs |
	sed -n 's/^ *\([a-z_]*\): Some(.*/\1/p' | sort -u)"

if [ "$fail" -eq 0 ]; then
	echo "process checks passed"
fi
exit "$fail"
