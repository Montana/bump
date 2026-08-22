#!/usr/bin/env bash

set -euo pipefail

# Regression tests for previously-broken behavior.
# Each block documents the bug it guards against.

source "$(dirname "$0")/lib.sh"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

pass() { echo "ok"; echo; }

fail() {
    echo "FAILED: $*"
    exit 1
}

run_ok() {
    local out status=0
    set +e
    out="$("$@" 2>&1)"
    status=$?
    set -e
    LAST_OUTPUT="$out"
    return $status
}

# Same, but with a working directory. Runs the subshell inside the command
# substitution so LAST_OUTPUT is still assigned in *this* shell.
run_ok_in() {
    local dir="$1"; shift
    local out status=0
    set +e
    out="$(cd "$dir" && "$@" 2>&1)"
    status=$?
    set -e
    LAST_OUTPUT="$out"
    return $status
}

# --------------------------------------------------------------------------
# mode/key migration: switching [base].mode must rewrite the component keys.
# Previously `set()` required the destination key to already exist, so any
# save after a mode switch failed with "Expected key 'major' not found" and
# left the file untouched -- after already printing a success line.
# --------------------------------------------------------------------------

echo "[mode-migration/calver-keys-under-semver]"
cp "$ROOT/tests/fixtures/malformed/semver-with-calver-keys.toml" "$WORK/m1.toml"
run_ok bump --patch "$WORK/m1.toml" || fail "bump --patch rejected a mode/key mismatch: $LAST_OUTPUT"
grep -q '^major = 2020' "$WORK/m1.toml" || fail "year was not rewritten to major"
grep -q '^patch = 2'    "$WORK/m1.toml" || fail "day was not rewritten to patch, or patch did not increment"
grep -q '^year = '      "$WORK/m1.toml" && fail "stale 'year' key left behind"
pass

echo "[mode-migration/semver-keys-under-calver]"
bump init "$WORK/m2.toml" >/dev/null
sed -i.bak 's/mode = "semver"/mode = "calver"/' "$WORK/m2.toml"
run_ok bump --calendar "$WORK/m2.toml" || fail "bump --calendar rejected a mode/key mismatch: $LAST_OUTPUT"
grep -q '^year = '  "$WORK/m2.toml" || fail "major was not rewritten to year"
grep -q '^major = ' "$WORK/m2.toml" && fail "stale 'major' key left behind"
pass

echo "[mode-migration/preserves-comments]"
bump init "$WORK/m3.toml" >/dev/null
sed -i.bak 's/^major = 0/# tracked component\nmajor = 0/' "$WORK/m3.toml"
sed -i.bak 's/mode = "semver"/mode = "calver"/' "$WORK/m3.toml"
run_ok bump --calendar "$WORK/m3.toml" || fail "$LAST_OUTPUT"
grep -q '# tracked component' "$WORK/m3.toml" || fail "comment lost during key rename"
pass

# --------------------------------------------------------------------------
# git independence: only --with-suffix/--full need git. Previously the suffix
# was resolved eagerly, so a repo with no commits (rev-parse HEAD fails) broke
# every command -- and `bump --patch` failed *after* bumping, losing the write.
# --------------------------------------------------------------------------

echo "[no-commits/print-does-not-need-git]"
mkdir -p "$WORK/fresh" && (cd "$WORK/fresh" && git init -q .)
bump init "$WORK/fresh/bump.toml" >/dev/null
run_ok_in "$WORK/fresh" bump print || fail "plain print failed in a commit-less repo: $LAST_OUTPUT"
[ "$LAST_OUTPUT" = "v0.1.0" ] || fail "expected v0.1.0, got '$LAST_OUTPUT'"
pass

echo "[no-commits/bump-persists]"
run_ok_in "$WORK/fresh" bump --patch || fail "bump --patch failed in a commit-less repo: $LAST_OUTPUT"
grep -q '^patch = 1' "$WORK/fresh/bump.toml" || fail "bump was reported but not persisted"
pass

echo "[no-commits/with-suffix-still-errors]"
run_ok_in "$WORK/fresh" bump print --with-suffix && fail "--with-suffix should fail without a resolvable HEAD"
pass

# --------------------------------------------------------------------------
# optional components: minor/patch may be omitted, but bumping an omitted
# component previously reported success and changed nothing.
# --------------------------------------------------------------------------

echo "[optional-components/bumping-absent-key-errors]"
bump init "$WORK/o1.toml" >/dev/null
sed -i.bak '/^patch = /d' "$WORK/o1.toml"
run_ok bump --patch "$WORK/o1.toml" && fail "bumping an absent 'patch' key silently succeeded"
[[ "$LAST_OUTPUT" == *"no 'patch' key"* ]] || fail "unhelpful error: $LAST_OUTPUT"
pass

# --------------------------------------------------------------------------
# malformed [timestamp].format used to panic inside chrono's Display impl.
# --------------------------------------------------------------------------

echo "[timestamp/invalid-format-errors-not-panics]"
bump init "$WORK/t1.toml" >/dev/null
sed -i.bak 's|^format = .*|format = "%Y-%Q"|' "$WORK/t1.toml"
run_ok bump --patch "$WORK/t1.toml" && fail "invalid strftime format was accepted"
[[ "$LAST_OUTPUT" == *"Invalid [timestamp].format"* ]] || fail "expected a clean error, got: $LAST_OUTPUT"
[[ "$LAST_OUTPUT" == *"panicked"* ]] && fail "still panicking: $LAST_OUTPUT"
pass

# --------------------------------------------------------------------------
# overflow used to panic (debug) / wrap silently (release).
# --------------------------------------------------------------------------

echo "[overflow/major-errors-not-panics]"
bump init "$WORK/v1.toml" >/dev/null
sed -i.bak 's/^major = 0/major = 4294967295/' "$WORK/v1.toml"
run_ok bump --major "$WORK/v1.toml" && fail "overflow was accepted"
[[ "$LAST_OUTPUT" == *"overflow"* ]] || fail "expected an overflow error, got: $LAST_OUTPUT"
[[ "$LAST_OUTPUT" == *"panicked"* ]] && fail "still panicking: $LAST_OUTPUT"
pass

# --------------------------------------------------------------------------
# `bump update pyproject.toml` used to exit 0 without writing anything when
# the file had no [project] table.
# --------------------------------------------------------------------------

echo "[update/pyproject-without-project-table-errors]"
bump init "$WORK/p1.toml" >/dev/null
printf '[tool.poetry]\nname = "x"\nversion = "0.0.1"\n' > "$WORK/pyproject.toml"
run_ok_in "$WORK" bump update pyproject.toml "$WORK/p1.toml" \
    && fail "missing [project] table silently reported success"
grep -q '0.0.1' "$WORK/pyproject.toml" || fail "file was modified despite the error"
pass

# --------------------------------------------------------------------------
# a prefix beginning with '-' used to be parsed by git as an option.
# --------------------------------------------------------------------------

echo "[tag/hyphen-prefix-is-not-a-git-option]"
mkdir -p "$WORK/tagrepo"
(
    cd "$WORK/tagrepo"
    git init -q .
    git -c user.email=t@t -c user.name=t commit -q --allow-empty -m init
)
bump init "$WORK/tagrepo/bump.toml" >/dev/null
(cd "$WORK/tagrepo" && bump --prefix '--delete' >/dev/null)
run_ok_in "$WORK/tagrepo" bump tag && fail "hostile tag name was accepted"
[[ "$LAST_OUTPUT" == *"unknown option"* ]] && fail "tag name still parsed as a git option: $LAST_OUTPUT"
pass

echo "All regression tests passed."
