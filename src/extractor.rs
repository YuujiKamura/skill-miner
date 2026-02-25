use crate::parser;
use crate::types::{ClassifiedConversation, DomainCluster, KnowledgePattern, Role};
use anyhow::Result;
use cli_ai_analyzer::{prompt, AnalyzeOptions};

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
            context_parts.push(format!(
                "=== 会話 {} (id: {}) ===\n{}",
                i,
                &conv.summary.id[..8.min(conv.summary.id.len())],
                exchanges.join("\n---\n")
            ));
        }
    }

    let context = context_parts.join("\n\n");

    let prompt_text = format!(
        r#"以下は「{domain}」分野のClaude Code会話群から抽出した要約である。
この分野で繰り返し現れるパターン（手順、判断基準、設計原則、ツール使用法）を抽出せよ。

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
    let patterns: Vec<PatternEntry> = parse_json_array(&response)?;

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

fn sanitize_json(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\r' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn parse_json_array(response: &str) -> Result<Vec<PatternEntry>> {
    let sanitized = sanitize_json(response);
    let trimmed = sanitized.trim();
    let json_str = if let Some(start) = trimmed.find('[') {
        let end = trimmed.rfind(']').map(|i| i + 1).unwrap_or(trimmed.len());
        &trimmed[start..end]
    } else {
        trimmed
    };

    serde_json::from_str(json_str).map_err(|e| {
        let preview: String = response.chars().take(200).collect();
        anyhow::anyhow!(
            "Failed to parse patterns JSON: {}\nResponse: {}",
            e,
            preview
        )
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let end: String = s.chars().take(max).collect();
        format!("{}...", end)
    }
}
