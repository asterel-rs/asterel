---
title: Experimental protocol
description: clean public snapshot から paper-facing Asterel evaluation artifacts を作るための runbook。
---

この protocol は、paper-facing evaluation run の最低基準を説明します。通常の release check より厳しくしています。読者が claim を再現し、artifact boundary を確認し、implementation evidence と empirical evidence を区別できる必要があるからです。

## 1. snapshot を凍結する

記録するもの:

- public commit hash
- clean working tree status
- 関連する Rust、Node、pnpm、Python、OS、Docker versions
- `Cargo.lock`、`pnpm-lock.yaml`、fixture hashes、benchmark adapter hashes
- secrets を除いた exact config files
- model / provider names、可能なら API versions、sampling parameters、random seeds

public paper artifact は、local agent notes や internal reviews を含む private development history ではなく、clean snapshot repository に基づくべきです。

## 2. claims と metrics を事前登録する

full benchmarks を実行する前に、claim を反証可能な形で書きます。

| Claim type | Example metric |
|---|---|
| Memory continuity | recall accuracy, correction latency, stale-recall rate |
| Exposure control | private-memory leak rate, safe-block rate, false-block rate |
| Naturalness | verifier reason counts, human naturalness ratings, over-explanation rate |
| Affect calibration | context-sensitivity score, inappropriate-tone rate |
| Security containment | attack success rate, unsafe action completion rate, false-positive block rate |

結果を見た後に primary metric を変えないでください。exploratory analyses を含めてもよいですが、その場合は exploratory と明記します。

## 3. data を安全に準備する

許可される inputs:

- license が確認された public benchmark datasets
- repository に commit された synthetic fixtures
- study protocol の下で収集された consented human-study transcripts
- raw private memory、tenant / person IDs、secrets、provider payloads を除いた redacted logs

禁止される inputs:

- 明示的な consent のない private Discord logs
- raw relationship memory または private grounding context
- exploit details を含む unresolved security findings
- provider credentials、OAuth tokens、webhook signatures、pairing tokens
- local handoff prompts または operator notes

## 4. local implementation gates を実行する

empirical benchmarks の前に implementation evidence を実行します。

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check-all
cargo test
pnpm --dir docs build
cargo metadata --no-deps --format-version 1
```

その後、paper claim に関係する replay fixtures と architecture checks を含め、[evidence ledger](./evidence-ledger/) の claim-focused checks を実行します。

## 5. benchmark と ablation suites を実行する

各 benchmark suite で次を行います。

1. isolated workspace を作る。
2. benchmark の public または synthetic input だけを load する。
3. full-runtime baseline を実行する。
4. [ablation plan](./ablation-plan/) の planned ablation conditions を実行する。
5. raw machine-readable results は private review area に保存する。
6. aggregate metrics、public-safe examples、hashes、redacted failure taxonomies だけを公開する。

provider-backed runs には retry policy と failure handling を含めます。provider call の失敗は run artifact として記録し、黙って落としません。

## 6. Human evaluation protocol

human raters を使う場合:

- study scope と publication boundary について consent を得る。
- participant が study のために明示的に提供した場合を除き、real private memories を避ける。
- 可能なら raters に condition names を blind する。
- free-form vibes ではなく written rubric を使う。
- 可能なら item ごとに少なくとも二人の independent ratings を集める。
- inter-rater agreement を報告する。計算しない場合は理由を説明する。
- safety-critical violations と aesthetic preferences を分ける。

companion dialogue の rubric dimensions 例:

- preceding turn とのつながり
- 適切な response density
- AI identity honesty
- public / private distance calibration
- overexposure なしの memory relevance
- correction 後の repair behavior
- human experience を過剰主張しない emotional attunement

## 7. Security evaluation protocol

security benchmarks は contained environment で実行します。

- real credentials を使わない。
- production workspaces を使わない。
- real external side effects を起こさない。
- mocked または disposable tools だけを使う。
- network targets の explicit allowlist を使う。
- policy decisions と blocked-action reason codes を保存する。

attack success と false-positive safe-blocking の両方を報告します。すべてを block する system は、有用な companion runtime ではありません。

## 8. Artifact layout

推奨する public artifact structure:

```text
artifacts/
  README.md
  environment.json
  configs/
  fixtures-hashes.txt
  benchmark-adapters/
  results/
    aggregate.csv
    aggregate.json
    ablations.csv
    failure-taxonomy.md
  redaction-policy.md
```

private raw logs は review 中に存在することがありますが、synthetic または完全に redacted でない限り public artifact の一部ではありません。

## 9. 結果は控えめに報告する

すべての result section に次を含めます。

- 何を実行したか。
- 何を実行していないか。
- 既知の excluded tests または datasets。
- model / provider sensitivity caveats。
- その結果が implementation invariant、fixture-backed behavior、empirical benchmark conclusion のどれを支えるか。

benchmark data または adapter を再配布できない場合は、licensed reader が独立して run を再現できるだけの hashes、schema、commands を公開します。
