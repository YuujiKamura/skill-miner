use crate::parser;
use crate::types::{ClassifiedConversation, DomainCluster, KnowledgePattern, Role};
use crate::util;
use anyhow::Result;
use cli_ai_analyzer::{prompt, AnalyzeOptions};
use rayon::prelude::*;
use std::collections::HashMap;

/// Extract knowledge patterns from a domain cluster.
/// Reads full conversation content for deeper analysis.
pub fn extract_patterns(
    domain: &str,
    conversations: &[&ClassifiedConversation],
    options: &AnalyzeOptions,
) -> Result<DomainCluster> {
    // Build context from full conversations (limited to avoid token overflow)
    let mut context_parts = Vec::new();

    for (i, conv) in conversations.iter().take(20).enumerate() {
        let full_conv = parser::parse_conversation(&conv.summary.source_path)?;

        // Extract user-assistant pairs (first 10 exchanges per conversation)
        let mut exchanges = Vec::new();
        let mut user_msg = None;

        for msg in full_conv.messages.iter().take(20) {
            match msg.role {
                Role::User => {
                    user_msg = Some(truncate(&msg.content, 300));
                }
                Role::Assistant => {
                    if let Some(u) = user_msg.take() {
                        let a = truncate(&msg.content, 500);
                        exchanges.push(format!("U: {}\nA: {}", u, a));
                    }
                }
            }
        }

        if !exchanges.is_empty() {
            let mut header = format!(
                "=== 会話 {} (id: {}) ===",
                i,
                &conv.summary.id[..8.min(conv.summary.id.len())],
            );
            // Append tool usage metadata if available
            if !conv.summary.files_touched.is_empty() {
                let files: Vec<_> = conv.summary.files_touched.iter().take(10).map(|f| f.as_str()).collect();
                header.push_str(&format!("\nfiles: [{}]", files.join(", ")));
            }
            if !conv.summary.commands_used.is_empty() {
                let cmds: Vec<_> = conv.summary.commands_used.iter().take(5).map(|c| c.as_str()).collect();
                header.push_str(&format!("\ncmds: [{}]", cmds.join(", ")));
            }
            context_parts.push(format!(
                "{}\n{}",
                header,
                exchanges.join("\n---\n")
            ));
        }
    }

    let context = context_parts.join("\n\n");

    let prompt_text = format!(
        r#"以下は「{domain}」分野のClaude Code会話群から抽出した要約である。
各会話にはfiles(操作されたファイルパス)とcmds(実行されたコマンド)のメタデータが含まれる。
この分野で繰り返し現れるパターン（手順、判断基準、設計原則、ツール使用法）を抽出せよ。
ファイルパスやコマンドパターンも考慮し、具体的なワークフローを特定せよ。

JSON配列で返せ。各要素:
{{
  "title": "パターン名",
  "description": "何をするパターンか",
  "steps": ["手順1", "手順2"],
  "frequency": 出現回数の推定
}}

会話データ:
{context}"#
    );

    let response = prompt(&prompt_text, options.clone())?;
    let patterns: Vec<PatternEntry> = util::parse_json_response(&response)?;

    let knowledge_patterns: Vec<KnowledgePattern> = patterns
        .into_iter()
        .map(|p| KnowledgePattern {
            title: p.title,
            description: p.description,
            steps: p.steps,
            source_ids: conversations
                .iter()
                .map(|c| c.summary.id.clone())
                .collect(),
            frequency: p.frequency,
        })
        .collect();

    Ok(DomainCluster {
        domain: domain.to_string(),
        conversations: conversations.iter().map(|c| (*c).clone()).collect(),
        patterns: knowledge_patterns,
    })
}

/// Extract patterns from all domains in parallel using rayon.
/// Returns (clusters, extract_call_count).
pub fn extract_all_parallel(
    groups: &HashMap<String, Vec<&ClassifiedConversation>>,
    options: &AnalyzeOptions,
) -> Result<(Vec<DomainCluster>, usize)> {
    let entries: Vec<_> = groups.iter().collect();

    let results: Vec<Result<DomainCluster>> = entries
        .par_iter()
        .map(|(domain, convs)| {
            eprintln!("  {} ({} conversations)...", domain, convs.len());
            extract_patterns(domain, convs, options)
        })
        .collect();

    let mut clusters = Vec::new();
    for result in results {
        clusters.push(result?);
    }

    let call_count = clusters.len();

    // Sort by domain name for deterministic output
    clusters.sort_by(|a, b| a.domain.cmp(&b.domain));

    Ok((clusters, call_count))
}

#[derive(serde::Deserialize)]
struct PatternEntry {
    title: String,
    description: String,
    #[serde(default)]
    steps: Vec<String>,
    #[serde(default = "default_freq")]
    frequency: usize,
}

fn default_freq() -> usize {
    1
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let end: String = s.chars().take(max).collect();
        format!("{}...", end)
    }
}
