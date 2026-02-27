# skill-miner

**Automatically extract domain knowledge from Claude Code conversations and generate reusable agent skills.**

skill-miner reads your Claude Code conversation history, classifies conversations by domain, extracts recurring patterns, and deploys them as skill files that Claude Code can use in future sessions. It uses progressive time-window mining so only new conversations are processed on each run.

## Quick Start

```sh
# Install
cargo install --path .

# Mine your conversation history and deploy skills
skill-miner mine

# See what was generated
skill-miner list
```

## Requirements

- Rust toolchain (1.70+)
- [cli-ai-analyzer](https://github.com/YuujiKamura/cli-ai-analyzer) crate (sibling directory)
- Claude Code with conversation history in `~/.claude/projects/`

## How It Works

### Pipeline

```
 Conversation JSONL
        |
        v
 +--------------+
 |    Parse      |  Read ~/.claude/projects/**/conversations/*.jsonl
 +--------------+
        |
        v
 +--------------+
 |   Compress    |  Extract topics, tools used, files touched
 +--------------+
        |
        v
 +--------------+
 |   Classify    |  AI assigns domain from domains.toml master list
 +--------------+
        |
        v
 +--------------+
 |   Extract     |  AI finds recurring patterns (freq >= 2, max 3 per domain)
 +--------------+
        |
        v
 +--------------+
 |   Generate    |  Build .md skill files with YAML frontmatter
 +--------------+
        |
        v
 +--------------+
 |   Deploy      |  Write to ~/.claude/skills/
 +--------------+
```

### Progressive Mining

The `mine` command expands time windows incrementally (12h, then 24h steps) from the present into the past. Already-processed conversations are tracked in `mined_ids`, so re-running `mine` only processes new conversations. Mining stops early when a window yields mostly `misc` (unclassifiable) conversations.

### Skill Lifecycle

```
draft --> approved --> deployed --> (consolidate) --> rejected/kept
```

- **draft**: Freshly generated from extracted patterns
- **approved**: Reviewed and ready for deployment
- **deployed**: Active in `~/.claude/skills/`
- **rejected**: Removed during consolidation (low score, dormant)

### Scoring & Consolidation

Skills are scored based on:

| Factor | Weight | Description |
|---|---|---|
| Fire rate | 60% | How often the skill is invoked (normalized) |
| Pattern richness | 40% | Sum of pattern frequencies (normalized) |
| Productivity | 0.5-1.0x | Fraction of invocations followed by tool use |
| Dormancy | 0.2-1.0x | Penalty for skills never invoked after 7-14 days |

## Command Reference

### Core Pipeline

#### `mine` -- Mine conversations and deploy skills

```sh
skill-miner mine [OPTIONS]
```

Runs the full pipeline: parse, compress, classify, extract, generate, and deploy. This is the primary command for most users.

| Option | Default | Description |
|---|---|---|
| `--max-days` | 30 | How far back to look |
| `--max-windows` | unlimited | Maximum time windows to process |
| `--min-messages` | 4 | Minimum messages per conversation |
| `--min-significance` | 0.3 | Stop if non-misc ratio drops below this |
| `--dry-run` | - | Preview without writing files |
| `--sync` | - | Git commit & push after deployment |
| `--parallel` | 4 | Maximum parallel AI calls |
| `-d, --dir` | `./skill-drafts` | Drafts directory |

#### `scan` -- Show conversation statistics

```sh
skill-miner scan [OPTIONS]
```

| Option | Default | Description |
|---|---|---|
| `--days` | 30 | How far back to scan |
| `--min-messages` | 4 | Minimum messages per conversation |
| `--fast` | - | Quick preview from history.jsonl |
| `--project` | - | Filter by project path (substring) |

#### `classify` -- Classify conversations by domain

```sh
skill-miner classify [OPTIONS]
```

Uses AI to assign each conversation to a domain from the master list.

#### `extract` -- Extract knowledge patterns

```sh
skill-miner extract --input <FILE> [OPTIONS]
```

Finds recurring patterns from classified conversations (frequency >= 2, max 3 per domain).

#### `generate` -- Generate skill files

```sh
skill-miner generate --input <FILE> [OPTIONS]
```

Creates `.md` skill drafts with YAML frontmatter from extracted patterns.

### Lifecycle Management

#### `list` -- List skill drafts

```sh
skill-miner list [-d <DIR>]
```

Shows all drafts with their status (draft/approved/deployed/rejected), scores, and fire counts.

#### `approve` -- Approve drafts for deployment

```sh
skill-miner approve [NAMES...] [--all] [-d <DIR>]
```

#### `reject` -- Reject drafts

```sh
skill-miner reject <NAMES...> [-d <DIR>]
```

#### `deploy` -- Deploy to ~/.claude/skills/

```sh
skill-miner deploy [NAMES...] [--approved] [-d <DIR>]
```

#### `diff` -- Show changes between draft and deployed

```sh
skill-miner diff [NAME] [-d <DIR>]
```

#### `consolidate` -- Score skills and prune dormant ones

```sh
skill-miner consolidate [NAMES...] [--all] [OPTIONS]
```

Scores skills using invocation logs from conversation history. Skills below `--min-score` are rejected. Use `--refine` to let AI rewrite descriptions based on actual trigger phrases.

| Option | Default | Description |
|---|---|---|
| `--all` | - | Consolidate all skills |
| `--days` | 30 | Days of invocation logs to scan |
| `--min-score` | 0.1 | Reject skills below this score |
| `--dry-run` | - | Preview without writing |
| `--refine` | - | AI-refine descriptions from trigger phrases |

#### `prune` -- Remove low-quality drafts

```sh
skill-miner prune [--misc] [--rejected] [--duplicates] [-d <DIR>]
```

### Sharing & Portability

#### `export` -- Create a .skillpack bundle

```sh
skill-miner export <OUTPUT> [OPTIONS]
```

Packages skills into a portable `.skillpack` directory for sharing.

| Option | Default | Description |
|---|---|---|
| `--name` | `my-skills` | Bundle name |
| `--author` | - | Bundle author |
| `--description` | `Exported skill bundle` | Bundle description |
| `--approved-only` | - | Only export approved/deployed skills |
| `--include-context` | - | Include referenced memory files |
| `--public` | - | Export sanitized public bundle |
| `--both` | - | Export private + public bundles |

#### `import` -- Import from a .skillpack bundle

```sh
skill-miner import <BUNDLE_PATH> [-d <DIR>]
```

#### `verify` -- Check bundle integrity

```sh
skill-miner verify <BUNDLE_PATH>
```

#### `validate` -- Validate bundle structure and content

```sh
skill-miner validate <BUNDLE_PATH> [--public] [--fix]
```

### Analysis

#### `graph` -- Show skill dependency graph

```sh
skill-miner graph [-d <DIR>]
```

Analyzes markdown links, skill references, and project paths between skills, memory files, and CLAUDE.md.

#### `today` -- Show today's work timeline

```sh
skill-miner today [--project <FILTER>] [--search <TEXT>]
```

## Configuration

### Domain Customization

Edit `domains.toml` to define your own domain categories:

```toml
[[domain]]
name = "Backend"
slug = "backend"
keywords = ["API", "database", "REST", "GraphQL", "server"]

[[domain]]
name = "Frontend"
slug = "frontend"
keywords = ["React", "CSS", "UI", "component", "layout"]

# Catch-all (must be last)
[[domain]]
name = "Other"
slug = "misc"
keywords = []
```

Each domain needs:
- `name`: Display name (used in AI classification prompts)
- `slug`: Stable identifier (used for filenames and tracking)
- `keywords`: Hints for fuzzy matching when AI output doesn't exactly match

The last entry with `slug = "misc"` acts as the catch-all for unclassifiable conversations.

### Directory Layout

```
~/.claude/
  projects/           # Claude Code conversation history (input)
    <project>/
      conversations/
        *.jsonl
  skills/             # Deployed skill files (output)
    <slug>.md
  history.jsonl       # Session history (for scoring/consolidation)

./skill-drafts/       # Local draft workspace
  manifest.json       # Draft status, scores, mined_ids
  <slug>.md           # Generated skill drafts
```

## Contributing

1. Fork and clone
2. `cargo build` to verify the build
3. `cargo test` to run the test suite
4. Submit a PR

## License

[MIT](LICENSE)
