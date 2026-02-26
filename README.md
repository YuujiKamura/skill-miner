# skill-miner

Claude Codeの会話履歴からドメイン知識を自動抽出し、再利用可能なAgentスキルとして配備するCLIツール。

## 必要なもの

- Rust toolchain
- [cli-ai-analyzer](../cli-ai-analyzer) (同階層に配置)
- Claude Code (`~/.claude/projects/` に会話JSONLが存在すること)

## インストール

```sh
cargo install --path .
```

## 使い方

### mine — 会話を掘ってスキルを即配備

```sh
skill-miner mine
```

直近から過去に向かって時間窓を広げながら会話を分類・パターン抽出し、`~/.claude/skills/` に直接配備する。処理済み会話は `mined_ids` で記録されるため、2回目以降は未処理分のみ追加処理される。

主要オプション:

| オプション | デフォルト | 説明 |
|---|---|---|
| `--max-days` | 30 | 最大何日前まで遡るか |
| `--max-windows` | なし | 処理する時間窓の最大数 |
| `--min-messages` | 4 | 会話あたりの最低メッセージ数 |
| `--min-significance` | 0.3 | misc分類が多いウィンドウの打ち切り閾値 |
| `--dry-run` | - | 実際の書き込みをせず結果だけ表示 |
| `--sync` | - | 配備後にgit commit & push |

```
$ skill-miner mine
[window 0] 12h ago → 0h ago: 8 new conversations
  ツール設計 → 8
[window 1] 36h ago → 12h ago: 9 new conversations
  スプレッドシート → 4, Rust開発 → 3, 写真管理 → 1, ツール設計 → 1
[window 2] 60h ago → 36h ago: 0 new conversations → stopping

Deployed 6 skills to ~/.claude/skills
```

### consolidate — 使われないスキルを淘汰

```sh
skill-miner consolidate --all
```

会話履歴からスキルの発火ログを集計し、スコアリング。長期間未使用のスキルに休眠ペナルティを適用し、低スコアのものを自動でreject。

### その他のコマンド

```sh
skill-miner list -d ./skill-drafts    # スキル一覧と状態
skill-miner scan --days 7             # 会話統計
skill-miner prune --misc --rejected   # 不要スキル削除
skill-miner export ./bundle           # .skillpack バンドル作成
skill-miner import ./bundle           # バンドルから取り込み
skill-miner graph                     # スキル間の依存関係
```

## パイプライン

```
会話JSONL → parse → compress → classify → extract → generate → deploy
              │                    │          │
              │                 ドメイン分類    パターン抽出(最大3, freq≥2)
              │
           時間窓で漸増(12h→24h→24h...)
           mined_idsで重複排除
           misc比率が高ければ早期停止
```

## ライセンス

Private repository.
