#!/usr/bin/env bash
# `just verify-package` must never report OK for a check it did not run,
# and CI must run it.
#
# `verify-package <channel>` must fail when validation fails, and a check
# that is skipped must not read as a check that passed. The recipe used to
# print
# `SKIP: ruby not on PATH` followed by `homebrew ... OK` and exit 0, and to
# exit 0 when asked to verify a .deb / .rpm that had never been built, while
# no workflow ran it at all. This script pins both halves:
#
#   1. Behaviour. With the verification tool missing from PATH, or with no
#      package built, `verify-package` exits non-zero and prints no OK line.
#      `CT_VERIFY_STRICT=0` (the laptop escape hatch) exits 0 with a `SKIP:`
#      line and still prints no OK.
#   2. Wiring. `lint` runs `verify-packaging-sources`, which covers every
#      source-form channel, and publish-deb.yml / publish-rpm.yml verify the
#      package they build in a step that a dry run does not skip.
#
# No mocks of the recipe itself: it runs through the real `just` and the real
# Justfile. The PATH is narrowed to a scratch directory holding only `just`,
# `python3` and `bash`, because "the tool is not installed" can only be
# produced by removing it. Two cases add an always-succeeding stand-in for
# `dpkg-deb` / `rpm`: there the tool is present on purpose, so that the
# missing *package* is the only thing that can make the check fail.
#
# Wire-up: `just lint` runs this script.
#
# Exit codes: 0 every assertion held; 1 an assertion failed (printed first).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

for tool in just python3 bash; do
  command -v "${tool}" >/dev/null || fail "${tool} is not on PATH; run this from the dev shell"
done

SCRATCH="$(mktemp -d)"
trap 'rm -rf "${SCRATCH}"' EXIT
BIN="${SCRATCH}/bin"
mkdir -p "${BIN}" "${SCRATCH}/empty-dist"
for tool in just python3 bash; do
  ln -s "$(command -v "${tool}")" "${BIN}/${tool}"
done

# run_verify <extra-path-dir-or-empty> <strict-value-or-empty> <args...>
# Runs `just verify-package <args...>` with PATH limited to ${BIN} (plus the
# optional extra directory). Sets RC and OUT.
run_verify() {
  local extra="$1" strict="$2"
  shift 2
  local path="${BIN}"
  [[ -n "${extra}" ]] && path="${extra}:${path}"
  local -a envs=(PATH="${path}" HOME="${HOME}" TMPDIR="${TMPDIR:-/tmp}")
  [[ -n "${strict}" ]] && envs+=(CT_VERIFY_STRICT="${strict}")
  set +e
  OUT="$(cd "${REPO_ROOT}" && env -i "${envs[@]}" just verify-package "$@" 2>&1)"
  RC=$?
  set -e
}

# expect_failure <description> <reason-regex>
# The run must fail, print no OK line, and fail for the stated reason (so a
# typo that makes `just` itself error out cannot pass for the check).
expect_failure() {
  local what="$1" reason="$2"
  if [[ ${RC} -eq 0 ]]; then
    fail "${what}: exited 0, a check that did not run passed. Output:
${OUT}"
  fi
  if grep -q ' OK$' <<<"${OUT}"; then
    fail "${what}: printed an OK line for a check that did not run. Output:
${OUT}"
  fi
  grep -Eq -- "${reason}" <<<"${OUT}" || fail "${what}: failed, but not because ${reason}. Output:
${OUT}"
  echo "ok: ${what} fails (rc ${RC}) and prints no OK"
}

expect_skip() {
  local what="$1"
  [[ ${RC} -eq 0 ]] || fail "${what}: CT_VERIFY_STRICT=0 should exit 0, got ${RC}. Output:
${OUT}"
  grep -q 'SKIP:' <<<"${OUT}" || fail "${what}: no SKIP: line. Output:
${OUT}"
  if grep -q ' OK$' <<<"${OUT}"; then
    fail "${what}: printed OK although it was skipped. Output:
${OUT}"
  fi
  echo "ok: ${what} skips loudly under CT_VERIFY_STRICT=0 and prints no OK"
}

# --- 1. Behaviour -----------------------------------------------------------

run_verify "" "" homebrew
expect_failure "homebrew without ruby" "ruby is not on PATH"
run_verify "" "1" homebrew
expect_failure "homebrew without ruby, CT_VERIFY_STRICT=1" "ruby is not on PATH"
run_verify "" "0" homebrew
expect_skip "homebrew without ruby"

run_verify "" "" nix
expect_failure "nix without nix-instantiate" "nix-instantiate is not on PATH"
run_verify "" "0" nix
expect_skip "nix without nix-instantiate"

run_verify "" "" deb "${SCRATCH}/empty-dist"
expect_failure "deb without dpkg-deb" "dpkg-deb is not on PATH"
run_verify "" "" rpm "${SCRATCH}/empty-dist"
expect_failure "rpm without rpm" "rpm is not on PATH"

# The tools are present (stand-ins that succeed for any input); only the
# package is missing.
STUBS="${SCRATCH}/stubs"
mkdir -p "${STUBS}"
for tool in dpkg-deb rpm; do
  printf '#!%s\nexit 0\n' "$(command -v bash)" >"${STUBS}/${tool}"
  chmod +x "${STUBS}/${tool}"
done
run_verify "${STUBS}" "" deb "${SCRATCH}/empty-dist"
expect_failure "deb with no .deb built" "no \\.deb in "
run_verify "${STUBS}" "" rpm "${SCRATCH}/empty-dist"
expect_failure "rpm with no .rpm built" "no \\.rpm in "
run_verify "${STUBS}" "0" deb "${SCRATCH}/empty-dist"
expect_skip "deb with no .deb built"

# --- 2. Wiring --------------------------------------------------------------

DUMP="$(cd "${REPO_ROOT}" && just --dump --dump-format json)"
python3 - "${DUMP}" "${REPO_ROOT}/.github/workflows" <<'PY'
import json, re, sys
from pathlib import Path

recipes = json.loads(sys.argv[1])["recipes"]
workflows = Path(sys.argv[2])

def fail(msg):
    print(f"FAIL: {msg}", file=sys.stderr)
    sys.exit(1)

def body_text(name):
    # just's JSON dump gives each body line as a list of fragments: plain
    # strings, and nested lists for {{interpolations}}.
    out = []
    for line in recipes[name]["body"]:
        out.append("".join(p if isinstance(p, str) else "{{}}" for p in line))
    return "\n".join(out)

def deps(name):
    return {d["recipe"] for d in recipes[name]["dependencies"]}

if not re.search(r"^\s*just verify-packaging-sources\s*$", body_text("lint"), re.M):
    fail("`lint` does not run `just verify-packaging-sources`")
want = {"verify-homebrew", "verify-nix", "verify-aur", "verify-scoop", "verify-chocolatey"}
if not want <= deps("verify-packaging-sources"):
    fail(f"verify-packaging-sources misses {sorted(want - deps('verify-packaging-sources'))}")
for alias in want:
    channel = alias.removeprefix("verify-")
    if f"just verify-package {channel}" not in body_text(alias):
        fail(f"{alias} does not run `just verify-package {channel}`")
print("ok: lint -> verify-packaging-sources -> every source-form channel")

# publish-deb / publish-rpm: a step that runs `just verify-<channel> <dir>`
# after the build step, with no `if:` (so a dry run runs it too).
for channel in ("deb", "rpm"):
    wf = (workflows / f"publish-{channel}.yml").read_text()
    steps = re.split(r"^      - ", wf, flags=re.M)[1:]
    names = [s.splitlines()[0] for s in steps]
    build = next((i for i, n in enumerate(names) if n.strip() == f"name: Build .{channel}"), None)
    verify = [i for i, s in enumerate(steps) if re.search(rf"\bjust verify-{channel} \S", s)]
    if build is None:
        fail(f"publish-{channel}.yml has no 'Build .{channel}' step")
    if not verify:
        fail(f"publish-{channel}.yml never runs `just verify-{channel} <dir>`")
    v = verify[0]
    if v <= build:
        fail(f"publish-{channel}.yml verifies before it builds")
    if re.search(r"^        if:", steps[v], re.M):
        fail(f"publish-{channel}.yml's verify step is conditional; a dry run would skip it")
    print(f"ok: publish-{channel}.yml verifies the .{channel} it builds, dry runs included")
PY

echo "verify-package-no-vacuous-pass: all checks passed"
