# Persona Evaluation Harness

`scripts/eval/persona_eval.py` runs multi-turn persona regression tests against the Asterel Gateway (`POST /webhook`) and scores each transcript with a binary checklist judge.

## Files

- `scripts/eval/persona_eval.py`: conversation runner, binary judge, GCR computation, baseline/compare.
- `scripts/eval/scenarios.json`: 48 scenarios (12 categories x primary/paraphrase_1/paraphrase_2/adversarial).
- `scripts/eval/results/`: default output directory for result artifacts.

## Requirements

- Python 3.10+
- `requests`
- Running Gateway endpoint (`http://127.0.0.1:3000/webhook` by default)

Install dependency:

```bash
python -m pip install requests
```

## Environment Variables

- `GATEWAY_URL` (optional, default: `http://127.0.0.1:3000`)
- `GATEWAY_TOKEN` (required)
- `ASTEREL_API_KEY` or `OPENAI_API_KEY` (required for judge API)
- `ASTEREL_API_BASE_URL` or `OPENAI_BASE_URL` (optional, default: `https://api.openai.com/v1`)
- `JUDGE_MODEL` (optional; default resolves to gateway model header if available, else model env, else `gpt-4o-mini`)

## Usage

Run full suite:

```bash
python scripts/eval/persona_eval.py
```

Custom paths:

```bash
python scripts/eval/persona_eval.py \
  --scenarios scripts/eval/scenarios.json \
  --output scripts/eval/results/
```

Save run as baseline:

```bash
python scripts/eval/persona_eval.py --baseline
```

Compare with baseline:

```bash
python scripts/eval/persona_eval.py --compare scripts/eval/results/baseline_YYYYMMDDTHHMMSS+0000.json
```

Run subset for quick debugging:

```bash
python scripts/eval/persona_eval.py --limit 8
```

## Output

Each run writes `scripts/eval/results/<run_id>_results.json` containing:

- Per scenario:
  - `scenario_id`, `category`, `variant`, `turns` (full transcript)
  - `checklist` (all 12 items, 0/1)
  - `checklist_avg`
  - `hard_constraints_pass`
  - `critical_pass`
  - `gcr_pass`
  - metadata (`gateway_model`, `gateway_model_version`, `system_prompt_hash`, `judge_model`, `timestamp`)
- Run summary:
  - `overall_gcr = passed_scenarios / total_scenarios`

When `--compare` is set, an additional `<run_id>_compare.json` is written with per-scenario and overall deltas.

## GCR Rule

Scenario `gcr_pass` is true only if:

1. All scenario `hard_constraints` pass.
2. No critical failures are present.
3. Binary checklist average is `>= 0.75`.

Overall GCR is:

```text
passed_scenarios / total_scenarios
```

## Notes

- The harness is synchronous (`requests` only), intentionally simple and flat.
- The judge is binary-only; no Likert scoring or vibe score.
- If gateway response headers include model/system-prompt metadata, those values are captured in results.
