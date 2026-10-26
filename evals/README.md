# milim agent evals

A small, deterministic agent-quality suite. `run.mjs` drives a running milim
desktop app through the canonical `/control/v1` API, the same contract the
mobile companion uses. For each task sample it:

1. copies `tasks/<id>/repo` to a temporary directory, runs `git init`, runs
   the task's optional `setup` (for example to build commit history), and
   commits it as the baseline;
2. creates a sidebar thread bound to that folder with the selected model,
   approval mode **Open** (or **Review** for a task with an approval
   policy), privacy **Off**, and memory off;
3. sends the task prompt with `turn.send` and polls the run until it
   completes, fails, or hits the task timeout. A multi-turn task sends its
   turns one after another in the same thread, each after the previous run
   finished, and snapshots the workspace after every turn. A timed-out turn
   is stopped with `turn.stop`, and the runner waits up to 30 seconds for
   the run to reach a terminal status so the check never races an agent
   that is still editing files. Approval requests are approved, or denied
   when the task's policy says so;
4. reads the run ledger (`GET /control/v1/runs/{id}` and `/events`) and the
   thread timeline for metrics and the sample's trajectory (each turn's
   final answer and workspace snapshot, every tool call, every approval
   decision);
5. runs the task's check script against the temporary repository, with the
   trajectory file in `EVAL_TRAJECTORY`.

Everything uses Node 22+ built-ins. There are no npm dependencies.

## Run it against the desktop app

The suite always targets the real Tauri desktop app: a release build, or the
debug binary that `pnpm -C apps/desktop verify:tauri` builds (the integration job
uses that one). `/control/v1` is served by the Rust host inside the app, so a
browser tab or the Vite dev server is never a valid target.

1. Start milim and make sure the model you want to evaluate works in an
   ordinary chat.
2. Open **Settings > Mobile**, enable the mobile companion, and pair a device.
   Copy the control URL shown there and the paired device's bearer key.
3. Run the suite from the repository root:

```sh
export MILIM_CONTROL_URL="http://192.168.1.20:7390"   # the URL from Settings > Mobile
export MILIM_DEVICE_KEY="..."                         # paired device key; never commit it
export MILIM_EVAL_MODEL="provider:openrouter:anthropic/claude-sonnet-4.5"
node evals/run.mjs
```

```powershell
$env:MILIM_CONTROL_URL = "http://192.168.1.20:7390"
$env:MILIM_DEVICE_KEY = "..."
$env:MILIM_EVAL_MODEL = "codex:gpt-5"
node evals/run.mjs
```

Treat the device key as a secret. The runner sends it only in the
`Authorization` header, never prints it, and removes it, together with every
other variable whose name looks secret (`*KEY*`, `*TOKEN*`, `*SECRET*`), from
the environment of check scripts. Revoke the device under Settings > Mobile
when you are done.

The desktop app must stay running for the whole suite. Its window does not
need to be visible: Rust owns accepted turns.

### Headless: a debug build and its control file

A debug desktop build launched with `MILIM_E2E_CONTROL_FILE=<path>` writes
`{"api_url", "token"}` for its main listener to that file (mode 0600, removed
on exit; release builds do not compile this). `run.mjs` accepts the same
variable as `evals/smoke.mjs`, waits for the file, and waits for the app to
accept requests, so no pairing is needed:

```sh
export MILIM_HOME="$(mktemp -d)"                      # throwaway app state
export MILIM_E2E_CONTROL_FILE="$(mktemp -d)/control.json"
MILIM_REMOTE_BASE_URL="https://openrouter.ai/api/v1" MILIM_REMOTE_API_KEY="..." \
  apps/desktop/src-tauri/target/tauri-verify/debug/milim-desktop &
MILIM_EVAL_MODEL="anthropic/claude-haiku-4.5" node evals/run.mjs --tasks bugfix
```

A fresh `MILIM_HOME` has no providers. `MILIM_REMOTE_BASE_URL` and
`MILIM_REMOTE_API_KEY` configure the app's OpenAI-compatible fallback backend
at launch (the same variables the CLI uses, see `docs/wiki/models.md`), and a
thread model id that no configured provider claims is sent there as is, so
`MILIM_EVAL_MODEL` is the raw id the endpoint expects. Model-run shell
commands never inherit milim's secret-named `MILIM_*` variables, so the key
does not reach the agent. The fallback backend has no catalog metadata:
milim does not know the model's context window (in-run context compaction
never triggers) and reports cost only when the provider sends it. To evaluate
a provider exactly as users configure it (including Anthropic and Gemini
provider kinds, context windows, and pricing), add it under
**Settings > Providers** in the app you run against and use the
`provider:<id>:<model>` id the model picker shows.

`evals/ci/headless-evals.sh` wraps all of this for CI and local use: it
launches a fresh instance of the debug binary with its own `MILIM_HOME` and
control file, hands `MILIM_EVAL_PROVIDER_KEY` only to the app, passes its
arguments to `run.mjs`, and stops the app afterwards
(`MILIM_EVAL_PROVIDER_KEY=... MILIM_EVAL_MODEL=... bash evals/ci/headless-evals.sh --tasks bugfix --repeat 3`).

**Why not a server-only binary.** `milim serve` (`crates/milim-cli`)
mounts the same `/control/v1` routes, but its tool registry is the
built-ins plus the `milim-tools` filesystem tools rooted at one fixed
directory. The host tools desktop agent runs actually use (`shell` with
background processes, `edit_file`, `patch_file`, `grep`, `glob`,
`diagnostics`, LSP, and the per-thread folder binding) live in the desktop
crate (`apps/desktop/src-tauri/src/host_tools.rs` and `host_tools/`, about
6,700 lines). They do not use Tauri APIs, but `milim serve` can only
register them once they move into a crate both binaries depend on, and it
has no counterpart of the debug control file. Until that refactor, a
server-only run would measure a different tool set, so the suite targets
the debug desktop binary, which already runs headless in CI without
pairing.

| Variable | Meaning |
|---|---|
| `MILIM_CONTROL_URL` | Mobile/control URL from Settings > Mobile, or the desktop API URL. Required unless `MILIM_E2E_CONTROL_FILE` is set. |
| `MILIM_DEVICE_KEY` | Paired device bearer key for the mobile/control URL. |
| `MILIM_API_TOKEN` | Desktop API bearer token, instead of a device key. |
| `MILIM_E2E_CONTROL_FILE` | Instead of the three above: the `{api_url, token}` file a debug build writes. |
| `MILIM_EVAL_MODEL` | Thread model id exactly as the model picker stores it: a provider model id, or `codex:`, `claude:`, `opencode:`, or `pi:` for account runtimes. Required. |
| `MILIM_EVAL_TASKS` | Comma list of task ids or substrings, for example `bugfix,grep-quota-status`. |
| `MILIM_EVAL_REPEAT` | Samples per task (default 1), same as `--repeat`. |
| `MILIM_EVAL_REASONING_EFFORT` | Per-thread reasoning effort override for the model, such as `high`. |
| `MILIM_EVAL_TIMEOUT_SCALE` | Multiplies every task timeout, for slow local models. |
| `MILIM_EVAL_CONNECT_TIMEOUT_MS` | How long to wait for the control file and for the app to accept requests (default 60000). |
| `MILIM_EVAL_KEEP=1` | Keep each temporary repository and record its path in the results. |
| `MILIM_EVAL_ARCHIVE=1` | Archive each eval thread after it finishes. Otherwise threads stay in the sidebar as `[eval] <task>` (`#<sample>` with repeats) for inspection. |
| `MILIM_EVAL_ARTIFACT_NAME` | Artifact name mentioned in the CI job-summary line. |

Other entry points:

```sh
node evals/run.mjs --dry-run            # list the selected tasks; no app needed
node evals/run.mjs --tasks large-file   # same as MILIM_EVAL_TASKS
node evals/run.mjs --repeat 5           # five samples per task
node evals/run.mjs --self-test          # plumbing check with the built-in mock-echo model
node evals/run.mjs --out my-run.json    # choose the results path
node evals/run.mjs --fail-on-error      # exit 1 if any sample could not be driven
node evals/validate.mjs                 # grader validation; no app or model needed
```

`--self-test` creates one thread with the `mock-echo` adapter, waits for the
run, and reads its ledger. It checks the control API and credentials, not an
agent.

## Integration evals

The Desktop integration workflow (`.github/workflows/nightly.yml`) runs on pushes to `main` and manual dispatch, with no daily schedule. Its `desktop-macos-launch` job builds the
real Tauri debug binary and runs the control smoke. After that it launches a
fresh instance of the same binary (new `MILIM_HOME`) and runs a small, cheap
task subset with `run.mjs` over the control file, but only when both of these
are configured in the repository settings:

| Setting | Kind | Meaning |
|---|---|---|
| `MILIM_EVAL_PROVIDER_KEY` | secret | API key for an OpenAI-compatible endpoint. Required. |
| `MILIM_EVAL_MODEL` | variable | Raw model id at that endpoint, e.g. `anthropic/claude-haiku-4.5`. Required. |
| `MILIM_EVAL_PROVIDER_BASE_URL` | variable | Endpoint base URL; default `https://openrouter.ai/api/v1`. |
| `MILIM_EVAL_NIGHTLY_TASKS` | variable | Task filter; default `bugfix-paginate,bugfix-duration,readonly-discount-question,run-tests-inventory`. |

The key reaches only the app process, as `MILIM_REMOTE_API_KEY` (the step
uses `evals/ci/headless-evals.sh`, see the headless section above); the
runner never sees it. Without the secret or the variable the step writes
"Agent evals skipped" to the job summary and succeeds. Otherwise it uploads
`evals/results/nightly.json` as the `eval-results-nightly` artifact and
appends one summary line (passes, unfinished, timeouts, errors, tokens,
cost) to the job summary. The step fails only when a sample could not be
driven at all (`--fail-on-error`), never because the model failed a task.
The toolchain setup and debug build are the composite action
`.github/actions/desktop-debug-build`, shared with the regression gate.

`.github/workflows/evals.yml` stays manual (`workflow_dispatch`) for the full
suite on a self-hosted runner whose machine already runs a paired app; it
accepts a `repeat` input and never runs on pull requests.

### Regression gate

`.github/workflows/eval-gate.yml` runs on pull requests that touch the agent
harness (`crates/milim-agents`, `crates/milim-tools`, `crates/milim-inference`,
`crates/milim-server/src/agent_prompt.rs`, `crates/milim-server/src/control/`,
the server's agent routes, the desktop host tools, `evals/`, and the gate
itself) and on manual dispatch. It builds the same debug binary, runs a
cheap-model task subset with `--repeat 3` through
`evals/ci/headless-evals.sh`, and then runs
`compare.mjs <baseline> evals/results/gate.json --shared-only --fail-on-regression`
against the committed baseline for that model (see
[`baselines/README.md`](baselines/README.md) for the file name and how to
record one). The comparison goes to the job summary and the results to the
`eval-results-gate` artifact. The job fails when a sample could not be
driven, when a shared task regresses beyond noise, or when the suite-level
sign test is significant.

| Setting | Kind | Meaning |
|---|---|---|
| `MILIM_EVAL_PROVIDER_KEY` | secret | Same key as the nightly subset. |
| `MILIM_EVAL_GATE_MODEL` | variable | Raw model id for the gate; defaults to `MILIM_EVAL_MODEL`. |
| `MILIM_EVAL_PROVIDER_BASE_URL` | variable | Endpoint base URL; default `https://openrouter.ai/api/v1`. |
| `MILIM_EVAL_GATE_TASKS` | variable | Task filter; default `bugfix-paginate,run-tests-inventory,ci-log-timezone,multiturn-list-then-fix,multiturn-undo-last-change,approval-denied-generated-write`. |

The gate succeeds with a notice, before building anything, when the
secret, the model variable, or the baseline file is missing, so it is safe
to enable before a baseline exists. Pull requests from forks (which get no
secrets) skip the job entirely. Three samples per side is the smallest
setting where one task can regress significantly (3/3 to 0/3); for a
finer signal raise both the baseline's and the gate's repeat.

## Results

Each run writes `evals/results/<timestamp>.json` (ignored by git), rewritten
after every sample so an interrupted suite keeps its partial results, and
prints a Markdown table. The JSON has `schema_version: 2`, the model, host
identity, `repeat`, and:

- `tasks`: one record per sample (a task appears `repeat` times);
- `task_summary`: one row per task with `samples`, `passes`, `pass_fraction`,
  `pass_at_k`, per-status counts, and mean metrics, including a
  `mean_<name>` for every waste metric below;
- `summary`: suite totals, including `total_redundant_reads`,
  `total_failed_edits`, `total_repeated_identical_calls`,
  `total_denied_calls`, `total_denied_retries`, and the means of
  `test_build_runs`, `calls_to_first_change`, `steps_to_first_change`, and
  `first_change_ms`.

The waste metrics come from the trajectory: tool names and arguments from
the timeline's `tool_call` items, outcomes from the matching `tool_result`
items, and steps and times from the ledger's `tool_result_committed`
events. Results recorded before they existed simply lack them; `compare.mjs`
shows `-` for that side.

Per sample record:

| Field | Meaning |
|---|---|
| `status` | `pass`, `fail`, `unfinished`, `timeout`, `error`, or `skipped` (see below). |
| `check_passed` | The check's own verdict, independent of how the run ended. |
| `run_status` | Final run status: `completed`, `failed`, `cancelled`, or `interrupted`; for a multi-turn task, the first turn that did not complete. |
| `run_limited` | A run stopped at its step or time budget (reported as `completed` by the host). |
| `turns` | Per turn: `run_id`, `run_status`, `timed_out`, `stop_confirmed` (the run reached a terminal status within 30 s of `turn.stop`), `limited`, `run_ms`, `started_at_ms`, `error`, and `answer` (the turn's final assistant text, first 2,000 characters). |
| `flags` | Why a sample is not a clean result, e.g. `turn 1: run failed`. |
| `thread_id`, `run_id` | Canonical ids for inspecting the (last) run in the app. |
| `check` | Check exit code, duration, and its first 2,000 characters of output. |
| `wall_ms`, `run_ms` | Runner wall time and the runs' own created-to-completed time. |
| `metrics.steps` | Model steps (distinct ledger steps with a provider request). |
| `metrics.tool_calls`, `tool_errors`, `tools` | Tool results in total, those that returned an error, and both per tool name. |
| `metrics.tool_argument_rejections` | Calls the agent loop refused before running: unknown tool, arguments that are not valid JSON, or arguments that fail the tool's schema. |
| `metrics.*_tokens`, `cached_input_tokens`, `cache_write_tokens`, `cost_usd` | Summed provider usage; cached input is already counted in `prompt_tokens`. Cost is provider-reported or estimated, falling back to the assistant message metrics. |
| `metrics.retries`, `retries_by_reason` | Provider retries (from `model_timing.attempts` or `provider_retry` timeline items) and their count per retry reason. |
| `metrics.context_compactions`, `context_elided_tool_results`, `context_summarized_messages` | In-run context management (`context_compacted` ledger events). |
| `metrics.output_limit_finishes`, `output_limit_continuations` | Steps that ended at the output-token limit, and those the loop continued with another step. |
| `metrics.model_latency_p50_ms`, `p95`, `first_token_p50_ms` | Model step latency from `model_timing`, else request/response timestamps. |
| `metrics.approvals_requested`, `auto_approved` | Approval prompts seen, and those the runner approved: any that still appear in Open mode, and in a task with an approval policy every request its deny rules do not match. |
| `metrics.redundant_reads` | `read_file` calls for the same path and range as an earlier read in the same turn, with no write or edit to that path in between. |
| `metrics.failed_edits` | `write_file`/`edit_file`/`patch_file` (and account-runtime equivalents) whose result was an error; denied calls are not counted. |
| `metrics.repeated_identical_calls` | Calls with the same tool and arguments as the call directly before them. |
| `metrics.test_build_runs` | Shell commands that run tests, a build, or a type check (`npm test`, `node --test`, `cargo test`, `tsc`, `pytest`, ...). |
| `metrics.denied_calls`, `denied_retries` | Calls answered with a denial, and later calls that repeat one (same tool and command or path, or matching the same deny rule). |
| `metrics.calls_to_first_change`, `steps_to_first_change`, `first_change_ms` | Tool calls, model steps (numbered across turns), and milliseconds from the first run's start until the first successful file-tool write; null when nothing was written through a file tool. |
| `trajectory` | Path of the sample's trajectory file, only with `MILIM_EVAL_KEEP=1`. |

A sample is `pass` only when the check passes **and** every turn's run
completed normally. `unfinished` means the check passes but a run failed, was
cancelled or interrupted, or stopped at its budget; such samples are listed
under "Flagged samples" and do not count as passes. `timeout` means a turn hit
the task deadline (whatever the check says), `error` means the runner could
not drive the task, and `skipped` means the machine lacks a task requirement
such as `python3`, `cargo`, or `tsc`. Pass rates exclude skipped samples.

Account runtimes (`codex:`, `claude:`, `opencode:`, `pi:`) record only a
harness boundary in the ledger, so steps, tool calls, and latency are often
unavailable for them; the pass/fail result is still meaningful.

### Repeats and pass@k

Model output varies from run to run, so one sample per task says little about
a single task. With `--repeat N`, `task_summary` reports each task's pass
fraction (passes / samples) and `pass_at_k` for every k up to N, using the
unbiased estimator `1 - C(n - c, k) / C(n, k)` for c passes in n samples:
the chance that at least one of k samples passes. `summary.mean_pass_at_1`
equals the mean pass fraction; `summary.mean_pass_at_repeat` is the share of
tasks solved at least once. Samples run round-robin (every task once, then
every task again), so an interrupted suite still covers each task evenly.

## Compare runs

```sh
node evals/compare.mjs evals/results/<baseline>.json evals/results/<candidate>.json
node evals/compare.mjs --a base-1.json base-2.json --b cand-1.json cand-2.json [--alpha 0.05] [--fail-on-regression] [--shared-only]
```

Each side may be several result files (repeated suites or `--repeat` runs,
schema 1 or 2); their samples are pooled per task. The per-task table shows
passes/samples on each side and marks a change only when it is beyond noise:

- **Per task**, a one-sided Fisher exact test on the 2x2 table of passes and
  failures asks how likely a drop at least this large is if both sides had
  the same pass rate. `REGRESSED` (or `IMPROVED`) needs `p <= alpha`
  (default 0.05); smaller moves show as `down (noise)` or `up (noise)`.
  With one sample per side, even pass to fail has `p = 0.5`, so a single
  task can never regress significantly; 3/3 to 0/3 is the smallest clear
  drop (`p = 0.05`), and 5/5 to 2/5 gives `p = 0.08`.
- **Suite level**, each shared task is one paired observation: an exact
  one-sided sign test compares the number of tasks whose pass rate went down
  with those that went up. This works even with one sample per task, since
  it needs many tasks to move the same way (for example 5 down and 0 up gives
  `p = 0.03`).

`--fail-on-regression` exits 1 when any task is `REGRESSED` or the suite-level
sign test is significant. Per-task tests are not corrected for multiple
comparisons; with 34 tasks at `alpha = 0.05`, rerun a flagged task before
acting on it, or pass a smaller `--alpha`. `--shared-only` lists only the
tasks both sides ran (the regression gate compares a subset against a
full-suite baseline); the suite-level test always uses shared tasks. The
aggregate table adds deltas for pass rate, unfinished runs, timeouts,
steps, tool calls and errors, argument rejections, context compactions,
retries, tokens (and cached tokens), cost, wall time, and the trajectory
waste metrics (redundant reads, failed edits, repeated identical calls,
test/build runs, denied-call retries, steps and seconds to the first
change) over the tasks both sides share.

## Tasks

| Category | Tasks |
|---|---|
| Bug fix with a failing test | `bugfix-paginate`, `bugfix-duration` |
| Implement to a spec | `implement-slugify`, `implement-ttl-cache` |
| Refactor across three files | `refactor-extract-helper`, `refactor-signature` |
| Find and fix in a ~45-file repository | `grep-quota-status`, `grep-cache-config` |
| Precise edit in a 1,500+ line file | `large-file-catalog-edit`, `large-file-function-fix` |
| One correct occurrence among identical lines | `ambiguous-edit-retry-config` |
| Follow the repository's AGENTS.md | `agents-md-conventions`, `agents-md-generated-code` |
| Run the tests and fix the failure | `run-tests-inventory`, `run-tests-async-queue` |
| Recover from a failing first command | `tool-error-missing-build` |
| Read-only question answered in a file | `readonly-discount-question`, `readonly-unused-exports` |
| Multi-step work that benefits from a todo list | `multistep-task-priority`, `multistep-config-v2` |
| Two turns, the second building on the first | `multiturn-phone-dedupe` |
| Later turns depend on what earlier turns listed, changed, or started | `multiturn-list-then-fix`, `multiturn-undo-last-change`, `multiturn-dev-server-session`, `multiturn-continue-migration` |
| Python (standard library only) | `python-invoice-totals` |
| TypeScript, graded with `tsc --noEmit` and tests | `typescript-exhaustive-refund` |
| Rust, graded with `cargo test` and hidden tests | `rust-release-registry` |
| Debug a CI-only failure from its pasted log | `ci-log-timezone` |
| Write tests, graded by seeded mutants | `test-writing-mutants` |
| Reach the goal another way after an approval is denied | `approval-denied-git-restore`, `approval-denied-generated-write` |
| Start, query, and stop a background process | `background-server-query` |
| Large context | `large-context-late-fees` |

Notes on the newer tasks:

- `multiturn-phone-dedupe` asks for a phone normalizer in turn 1; turn 2 asks
  to reuse "the function you just wrote" without naming it again.
- `ambiguous-edit-retry-config` has ten identical
  `retry: { attempts: 3, backoffMs: 250 },` lines, and the payments block is
  the same in both environments apart from its URL; only production payments
  may change.
- `python-invoice-totals` needs Python 3.8+ (`python3`, or `python` when it is
  Python 3). Without it the runner records the task as `skipped` and its check
  exits 77 with `SKIP python3 is not installed`.
- `tool-error-missing-build`: the natural first `npm test` fails because the
  rate table is generated and not checked in, and the build script refuses to
  run without flags. The check rebuilds the table from the agent's build
  script, so the root cause (a `Number("0") || 2` in the script) must be fixed
  there, not in the generated file.
- `background-server-query`: the service takes 1.5 s to start, picks a free
  port, and prints its URL; `request_id` is random per request, so the answer
  can only come from a live query. The check also fails (and kills the
  process) if the service is still running. The desktop shell tool keeps
  `run_in_background` processes for the chat's later turns, so milim's own
  agent loop must stop the service with `process_kill`, like account
  runtimes.
- `large-context-late-fees`: fourteen weeks of meeting notes (~650 KB, about
  160k tokens by milim's chars/4 estimate) with the late-fee rules scattered
  through them, some revised in later weeks, among filler that reuses the
  same keywords. Reading every file crosses milim's pruning threshold (older
  tool output is elided above 60% of the context window, turns are
  summarized above 85%) for windows up to roughly 270k tokens, so models with
  128k to 200k windows must keep track of rules they read before compaction.
  A 1M-token model can hold everything and usually records no compaction,
  and a model that greps or scripts instead of reading reads far less. Compare
  `metrics.context_compactions` alongside the result, and only compare this
  task between models with similar windows. The fixture is generated by
  `evals/fixtures-gen/meeting-notes.mjs`.
- `multiturn-list-then-fix`: turn 1 lists the three failing tests without
  changing anything, turn 2 fixes "only the second failure on your list",
  turn 3 "the first one" too. The check reads the turn-1 answer, orders the
  failures by where the answer first mentions each, and checks each turn's
  snapshot against that order, so an agent that counts in the test
  runner's order instead of its own fails.
- `multiturn-undo-last-change`: turn 1 renames a setting (touching
  src/config.js), turn 2 bumps a default in the same file and the docs,
  turn 3 says "undo the last change you made to src/config.js". The file
  must end exactly as turn 1 left it (whitespace aside) with the docs as
  turn 2 left them; `git checkout` of the file also reverts the rename.
- `multiturn-dev-server-session`: turn 1 starts a dev server in the
  background and reports its random boot id, turn 2 queries "the server
  that is already running" (a counter that starts at a random value) and
  reports the boot id again, turn 3 stops it. Two boots, a different boot
  id, or a counter value the server never returned fail. milim's `shell`
  tool keeps a chat's background processes across its turns, so this task
  checks that the agent reuses the turn-1 server (found in the per-turn
  context and the replayed turn) instead of starting another.
- `multiturn-continue-migration`: twelve callback-style handlers to convert
  to `async`; turn 1 converts the a-m half (with a router that accepts
  both styles, tests green on the turn-1 snapshot) and lists the rest, and
  turn 2 is only `continue`.
- `typescript-exhaustive-refund` needs `tsc` (on PATH, or the desktop or
  site app's installed `typescript`; the suite adds no npm dependency) and a
  Node that strips TypeScript types by default (22.18+, 23.6+). For agent
  runs, put `tsc` on the app's PATH so the agent can run
  `npm run typecheck` itself. Casts, non-null assertions, `any`, and
  `@ts-` comments fail the check.
- `rust-release-registry` needs `cargo`. A borrow error hides a derived
  `Ord` that compares fields in declaration order (patch first); the
  visible test only sorts, the hidden tests (written into `tests/` by the
  check and removed afterwards) also compare versions and pick `latest`.
- `ci-log-timezone`: the log shows the job runs with
  `TZ=America/New_York`; the code buckets UTC timestamps with local-time
  getters, which passes at UTC to UTC+2. The check runs the tests and
  hidden cases under five time zones and rejects forcing `TZ`.
- `test-writing-mutants`: the agent writes the tests; they must pass on
  the real module and fail on each of ten seeded mutants (each breaks one
  documented behavior).
- `approval-denied-git-restore` (history built by `setup.mjs`) and
  `approval-denied-generated-write` carry an approval policy: they run in
  Review mode, the runner denies history-rewriting git commands and file
  writes under `src/generated/` respectively, and approves everything else.
  The checks grade the goal (tests, hidden cases, history unchanged, the
  generated file matching its generator) and fail when the agent asks
  again for a denied action. If the agent never tries the denied action,
  the task still passes on the goal alone.

Each task directory contains:

- `task.json`: `id` (matches the directory), `title`, `category`,
  `timeout_s`, `check` (normally `node check.mjs`), and either `prompt` or
  `turns` (a list of `{ "prompt", "timeout_s" }` sent in order to the same
  thread; `timeout_s` defaults to the task's). Optional:
  - `requires`: machine requirements (`python3`, `cargo`, `typescript`);
  - `setup`: a command (resolved against the task directory like `check`)
    run in the fresh repository after `git init`, for example to create
    commit history; what it leaves uncommitted becomes the baseline commit;
  - `approvals`: `{ "mode": "review", "deny": [{ "tool", "pattern", "first_only" }] }`.
    The thread runs in `mode` (default `review`, so consequential calls ask
    at all). A request is denied when a rule matches: `tool` is a tool name
    or list of names (omitted: any tool), `pattern` a regular expression
    tested against the command of a shell call, the path of a file call, or
    the JSON arguments otherwise, and `first_only` denies only the first
    match. Every other request is approved. Rules match the tool names the
    runtime reports, so they target milim's own loop (`shell`, `edit_file`,
    `write_file`, `patch_file`);
- `repo/`: the fixture copied into the temporary workspace;
- `check.mjs`: the deterministic grader;
- `solution.patch` or `solution.mjs`: the reference solution;
- optional `alt-<name>.patch|.mjs` (other correct solutions) and
  `wrong-<name>.patch|.mjs` (plausible wrong ones).

The check runs with its working directory set to the temporary repository.
Arguments in `check` that name a file in the task directory resolve there, so
the grader and solutions stay outside the agent's workspace and cannot be
edited by it. Checks import `evals/lib/check.mjs`, whose helpers read the
committed baseline (`headFile`, `changedFiles`, `expectOnlyChanged`,
`changedLineCounts`), run the repository's own `node:test` files (optionally
in another directory or with extra environment), load modules fresh (or from
source text with `loadSource`), strip comments before a source scan
(`stripComments`), and `skip` when a requirement is missing. A check exits 0
on pass, 77 on skip, and prints `FAIL <reason>` lines otherwise.

The trajectory (`EVAL_TRAJECTORY`, format in `lib/trajectory.mjs`) lets a
check grade what the files alone cannot: `turnAnswer(n)` is turn n's final
answer, `turnFile(n, path)` and `turnChangedFiles(n)` read the workspace as
turn n left it (a git tree written with a throwaway index, so the agent's
own index and history are untouched), `materializeTurn(n)` extracts that
snapshot to run tests on it, and `trajectory().tool_calls` / `.approvals`
list every call and approval decision. `requireTrajectory()` fails a check
that needs one when it is missing, which is the case for the untouched
fixture under `validate.mjs`.

The medium and large fixtures (`grep-*`, `large-file-*`, `large-context-*`)
are generated by `evals/fixtures-gen/` and checked in so every task is
self-contained. After editing a generator, run
`node evals/fixtures-gen/generate.mjs`; `--check` reports drift without
writing.

### Grader validation

```sh
node evals/validate.mjs [--tasks a,b] [--repeat N] [--strict] [--verbose]
```

For every task, without a model, `validate.mjs` checks in fresh temporary
copies that the check **fails** on the untouched fixture, **passes** after
the reference solution (a `.patch` is applied with `git apply`; a `.mjs` runs
with the repository as its working directory), passes for every `alt-*`
solution, and fails for every `wrong-*` one. A `.mjs` solution for a
multi-turn or approval task simulates the run: it edits the repository and
calls `recordTurn({ answer, toolCalls, approvals })` from
`lib/trajectory.mjs` at the end of each simulated turn, which snapshots the
workspace into the trajectory file the check then reads. Helper modules a
task's solutions share (for example `fixes.mjs`) live next to them; only
`solution.*`, `alt-*`, and `wrong-*` files are run. `--repeat` reruns each passing
check to catch flaky graders, and `--strict` treats a task skipped for a
missing requirement as a failure. CI runs it with `--strict` on every pull
request (the `Eval graders` job in `ci.yml`), together with the fixture drift
check and `run.mjs --dry-run`.

When adding a task, write its reference solution first and make sure
`validate.mjs` passes: the check must fail on the untouched fixture and pass
after a correct fix, even one that differs from yours (add it as `alt-*`).
Avoid timing-dependent assertions, allow every file a correct fix may touch,
and restrict everything else with `expectOnlyChanged` so unrelated edits fail.
A `wrong-*` solution for the most likely mistake keeps the grader honest.

## Limitations

- The runner needs a running desktop app; it does not start one.
  `evals/ci/headless-evals.sh` (used by the integration job and the
  regression gate) launches the debug binary around it.
- Approval mode Open gives the agent full access within the temporary
  workspace and runs host commands without asking, and Review tasks
  approve everything their deny rules do not match. Run the suite only on a
  machine you trust with the model you are testing.
- Waste metrics need the timeline's `tool_call` items; account runtimes
  that project only results report calls without arguments, so read-based
  and repeat-based counts are lower bounds for them.
- Privacy is set to Off for eval threads so fixtures are sent verbatim.
- Checks need `git`, `tar`, and Node 22+ on the machine running the suite,
  `python3` for the Python task, `cargo` for the Rust task, and `tsc` plus
  a type-stripping Node (22.18+) for the TypeScript task. CI's grader job
  installs the pinned `typescript` globally for `--strict`.
- Reference solutions and checks live in the task directory, outside the
  agent's workspace but on the same machine; an agent that searches the whole
  disk could find them.
