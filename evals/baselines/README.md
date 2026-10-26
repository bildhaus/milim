# Eval baselines

Committed `run.mjs` result files that the pull-request regression gate
(`.github/workflows/eval-gate.yml`, see "Regression gate" in
`evals/README.md`) compares against. Unlike `evals/results/`, this directory
is tracked by git.

There is one file per model, named after the model id with every character
outside `[A-Za-z0-9._-]` replaced by `_`:

| `MILIM_EVAL_MODEL` | Baseline file |
|---|---|
| `anthropic/claude-haiku-4.5` | `evals/baselines/anthropic_claude-haiku-4.5.json` |
| `provider:openrouter:openai/gpt-5-mini` | `evals/baselines/provider_openrouter_openai_gpt-5-mini.json` |

When the file for the gate's model does not exist, the gate skips with a
notice instead of failing.

## Producing a baseline

Record it from `main` (or the commit the change is based on), with the same
model id and provider endpoint the gate uses, and at least the gate's task
subset (`MILIM_EVAL_GATE_TASKS`; the whole suite is fine, since the gate
compares only the tasks both sides ran). Use three or more samples per task:
with one sample per side no single task can regress significantly.

Headless, against a fresh debug build (`pnpm -C apps/desktop verify:tauri`),
exactly as CI runs it:

```sh
export MILIM_EVAL_PROVIDER_KEY="..."              # never commit it
export MILIM_EVAL_MODEL="anthropic/claude-haiku-4.5"
export EVAL_PROVIDER_BASE_URL="https://openrouter.ai/api/v1"
bash evals/ci/headless-evals.sh --repeat 3 --out evals/baselines/anthropic_claude-haiku-4.5.json
```

Or against a running, paired desktop app:

```sh
MILIM_CONTROL_URL=... MILIM_DEVICE_KEY=... MILIM_EVAL_MODEL=... \
  node evals/run.mjs --repeat 3 --out evals/baselines/<model>.json
```

Check the result before committing it: no `error` samples, the commit and
app you ran against noted in the pull request, and nothing secret in it (the
runner never writes credentials; samples contain task ids, statuses,
metrics, and the agent's final answer per turn).

## How it is used

The gate runs its subset with `--repeat 3` and then

```sh
node evals/compare.mjs evals/baselines/<model>.json evals/results/gate.json --shared-only --fail-on-regression
```

which fails when a shared task drops beyond noise (one-sided Fisher exact
test, for example 3/3 to 0/3) or when significantly more tasks went down
than up (suite-level sign test). The comparison, including the trajectory
waste metrics, goes to the job summary and the `eval-results-gate` artifact.

Refresh a baseline in the same pull request as a change that is expected to
move results (a harness change that trades one task for another, a new
task, a prompt rewrite), and say why in the description. Never edit a
baseline by hand, and never commit numbers that did not come from a run.
