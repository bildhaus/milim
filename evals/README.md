# milim agent evals

A small, deterministic agent-quality suite. `run.mjs` drives a running milim
desktop app through the canonical `/control/v1` API, the same contract the
mobile companion uses. For each task it:

1. copies `tasks/<id>/repo` to a temporary directory, runs `git init`, and
   commits it as the baseline;
2. creates a sidebar thread bound to that folder with the selected model,
   approval mode **Open**, privacy **Off**, and memory off;
3. sends the task prompt with `turn.send` and polls the run until it
   completes, fails, or hits the task timeout (a timed-out turn is stopped
   with `turn.stop`);
4. runs the task's check script against the temporary repository;
5. reads the run ledger (`GET /control/v1/runs/{id}` and `/events`) and the
   thread timeline for metrics.

Everything uses Node 22+ built-ins. There are no npm dependencies.

## Run it against the desktop app

1. Start milim (a release build or `pnpm -C apps/desktop tauri:dev`) and make
   sure the model you want to evaluate works in an ordinary chat.
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
`Authorization` header, never prints it, and clears it from the environment of
check scripts. Revoke the device under Settings > Mobile when you are done.

The desktop app must stay running for the whole suite. Its window does not
need to be visible: Rust owns accepted turns.

| Variable | Meaning |
|---|---|
| `MILIM_CONTROL_URL` | Mobile/control URL from Settings > Mobile. Required. |
| `MILIM_DEVICE_KEY` | Paired device bearer key. Required. |
| `MILIM_EVAL_MODEL` | Thread model id exactly as the model picker stores it: a provider model id, or `codex:`, `claude:`, `opencode:`, or `pi:` for account runtimes. Required. |
| `MILIM_EVAL_TASKS` | Comma list of task ids or substrings, for example `bugfix,grep-quota-status`. |
| `MILIM_EVAL_REASONING_EFFORT` | Per-thread reasoning effort override for the model, such as `high`. |
| `MILIM_EVAL_TIMEOUT_SCALE` | Multiplies every task timeout, for slow local models. |
| `MILIM_EVAL_KEEP=1` | Keep each temporary repository and record its path in the results. |
| `MILIM_EVAL_ARCHIVE=1` | Archive each eval thread after it finishes. Otherwise threads stay in the sidebar as `[eval] <task>` for inspection. |

Other entry points:

```sh
node evals/run.mjs --dry-run            # list the selected tasks; no app needed
node evals/run.mjs --tasks large-file   # same as MILIM_EVAL_TASKS
node evals/run.mjs --self-test          # plumbing check with the built-in mock-echo model
node evals/run.mjs --out my-run.json    # choose the results path
```

`--self-test` creates one thread with the `mock-echo` adapter, waits for the
run, and reads its ledger. It checks the control API and credentials, not an
agent.

## Results

Each run writes `evals/results/<timestamp>.json` (ignored by git) and prints a
Markdown table. The JSON has `schema_version: 1`, the model, host identity,
and one record per task:

| Field | Meaning |
|---|---|
| `status` | `pass`, `fail`, `timeout`, or `error` (the runner could not drive the task). |
| `thread_id`, `run_id`, `run_status` | Canonical ids for inspecting the run in the app. |
| `check` | Check exit code, duration, and its first 2,000 characters of output. |
| `wall_ms`, `run_ms` | Runner wall time and the run's own created-to-completed time. |
| `metrics.steps` | Model steps (distinct ledger steps with a provider request). |
| `metrics.tool_calls`, `tool_errors`, `tools` | Tool results in total, those that returned an error, and both per tool name. |
| `metrics.*_tokens`, `cost_usd` | Summed provider usage; cost is provider-reported or estimated, falling back to the assistant message metrics. |
| `metrics.retries` | Provider retries (from `model_timing.attempts`, else repeated requests in one step). |
| `metrics.model_latency_p50_ms`, `p95`, `first_token_p50_ms` | Model step latency from `model_timing`, else request/response timestamps. |
| `metrics.approvals_requested`, `auto_approved` | Approval prompts seen. The runner approves any that still appear in Open mode. |

Account runtimes (`codex:`, `claude:`, `opencode:`, `pi:`) record only a
harness boundary in the ledger, so steps, tool calls, and latency are often
unavailable for them; the pass/fail result is still meaningful.

## Compare two runs

```sh
node evals/compare.mjs evals/results/<baseline>.json evals/results/<candidate>.json
```

It prints a per-task table (pass/fail with `REGRESSED` and `fixed` markers,
plus step, tool-call, and token deltas) and aggregate deltas over the tasks
both files share: pass rate, mean steps, mean tool calls, tool errors, mean
tokens, total cost, and mean wall time. `--fail-on-regression` exits 1 when
any task went from pass to something else.

Model output varies between runs. Compare several runs per configuration
before trusting a small difference.

## Tasks

| Category | Tasks |
|---|---|
| Bug fix with a failing test | `bugfix-paginate`, `bugfix-duration` |
| Implement to a spec | `implement-slugify`, `implement-ttl-cache` |
| Refactor across three files | `refactor-extract-helper`, `refactor-signature` |
| Find and fix in a ~45-file repository | `grep-quota-status`, `grep-cache-config` |
| Precise edit in a 1,500+ line file | `large-file-catalog-edit`, `large-file-function-fix` |
| Follow the repository's AGENTS.md | `agents-md-conventions`, `agents-md-generated-code` |
| Run the tests and fix the failure | `run-tests-inventory`, `run-tests-async-queue` |
| Read-only question answered in a file | `readonly-discount-question`, `readonly-unused-exports` |
| Multi-step work that benefits from a todo list | `multistep-task-priority`, `multistep-config-v2` |

Each task directory contains:

- `task.json`: `id` (matches the directory), `title`, `category`, `prompt`,
  `timeout_s`, and `check` (normally `node check.mjs`);
- `repo/`: the fixture copied into the temporary workspace;
- `check.mjs`: the deterministic grader.

The check runs with its working directory set to the temporary repository.
Arguments in `check` that name a file in the task directory resolve there, so
the grader stays outside the agent's workspace and cannot be edited by it.
Checks import `evals/lib/check.mjs`, whose helpers read the committed baseline
(`headFile`, `changedFiles`, `expectOnlyChanged`), run the repository's own
`node:test` files, and load modules fresh. A check exits 0 on pass and prints
`FAIL <reason>` lines otherwise.

The medium and large fixtures (`grep-*`, `large-file-*`) are generated by
`evals/fixtures-gen/` and checked in so every task is self-contained. After
editing a generator, run `node evals/fixtures-gen/generate.mjs`; `--check`
reports drift without writing.

When adding a task, make sure its check fails on the untouched fixture and
passes after a correct fix, avoid timing-dependent assertions, and restrict
allowed changes with `expectOnlyChanged` so unrelated edits fail.

## Limitations

- The runner needs a running, paired desktop app; it does not start one.
- Approval mode Open gives the agent full access within the temporary
  workspace and runs host commands without asking. Run the suite only on a
  machine you trust with the model you are testing.
- Privacy is set to Off for eval threads so fixtures are sent verbatim.
- Checks need `git` and Node 22+ on the machine running the suite.
- The `.github/workflows/evals.yml` workflow is manual only and needs a
  self-hosted runner that already has a paired app; it never runs on pull
  requests.
