#!/usr/bin/env bash
# Run evals/run.mjs against a fresh, headless launch of the real Tauri debug
# binary (see "Headless" in evals/README.md), then stop the app. Used by the
# nightly subset and the pull-request regression gate.
#
#   MILIM_EVAL_PROVIDER_KEY=... MILIM_EVAL_MODEL=... \
#     bash evals/ci/headless-evals.sh --out evals/results/x.json --fail-on-error [--repeat 3]
#
# Arguments go to run.mjs unchanged. The provider key reaches only the app
# process, as MILIM_REMOTE_API_KEY for its OpenAI-compatible fallback backend
# (model-run shell commands never inherit secret-named MILIM_* variables);
# run.mjs never sees it.
#
#   MILIM_EVAL_PROVIDER_KEY   API key for the endpoint (required)
#   MILIM_EVAL_MODEL          raw model id at the endpoint (required)
#   EVAL_PROVIDER_BASE_URL    endpoint base URL (default https://openrouter.ai/api/v1)
#   MILIM_DESKTOP_BINARY      debug binary (default: the one `pnpm -C apps/desktop verify:tauri` builds)
set -euo pipefail

: "${MILIM_EVAL_PROVIDER_KEY:?set MILIM_EVAL_PROVIDER_KEY}"
: "${MILIM_EVAL_MODEL:?set MILIM_EVAL_MODEL}"
binary="${MILIM_DESKTOP_BINARY:-apps/desktop/src-tauri/target/tauri-verify/debug/milim-desktop}"
base_url="${EVAL_PROVIDER_BASE_URL:-https://openrouter.ai/api/v1}"
test -x "$binary" || { echo "desktop debug binary not found: $binary" >&2; exit 2; }

export MILIM_HOME="$(mktemp -d)"
e2e_dir="$(mktemp -d)"
chmod 700 "$e2e_dir"
export MILIM_E2E_CONTROL_FILE="$e2e_dir/control.json"
log="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/milim-desktop-evals-$$.log"

env -u MILIM_EVAL_PROVIDER_KEY \
  MILIM_REMOTE_BASE_URL="$base_url" \
  MILIM_REMOTE_API_KEY="$MILIM_EVAL_PROVIDER_KEY" \
  RUST_BACKTRACE=1 "$binary" >"$log" 2>&1 &
pid=$!

status=0
env -u MILIM_EVAL_PROVIDER_KEY MILIM_EVAL_ARCHIVE=1 node evals/run.mjs "$@" || status=$?

kill "$pid" 2>/dev/null || true
for _ in $(seq 1 20); do
  kill -0 "$pid" 2>/dev/null || break
  sleep 0.5
done
kill -9 "$pid" 2>/dev/null || true
rm -rf "$MILIM_HOME" "$e2e_dir"
if [ "$status" != 0 ]; then
  tail -n 200 "$log"
fi
rm -f "$log"
exit "$status"
