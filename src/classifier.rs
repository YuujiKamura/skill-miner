use crate::compressor;
use crate::domains;
use crate::types::{ClassifiedConversation, ConversationSummary};
use crate::util;
use anyhow::Result;
use cli_ai_analyzer::{prompt, AnalyzeOptions};

/// Classify conversation summaries into domain clusters using AI
pub fn classify(
    summaries: &[ConversationSummary],
    options: &AnalyzeOptions,
) -> Result<Vec<ClassifiedConversation>> {
    // Process in batches to stay within context limits
    let batch_size = 50;
    let mut all_classified = Vec::new();

    for batch in summaries.chunks(batch_size) {
        let text = compressor::format_for_classification(batch);
        let classified = classify_batch(batch, &text, options)?;
        all_classified.extend(classified);
    }

    Ok(all_classified)
}

fn classify_batch(
    summaries: &[ConversationSummary],
    formatted_text: &str,
    options: &AnalyzeOptions,
) -> Result<Vec<ClassifiedConversation>> {
    let domain_list = domains::prompt_domain_list();

    let prompt_text = format!(
        r#"以下はClaude Codeのチャット会話の一覧である。各会話を以下の分野リストから最も適切なものに分類せよ。
リスト外の分野名を使うな。必ず以下のいずれかを選べ:

{domain_list}

JSON配列で返せ。各要素: {{"index": 0, "domain": "分野名", "tags": ["tag1"], "confidence": 0.9}}

会話一覧:
{formatted_text}"#
    );

    let response = prompt(&prompt_text, options.clone())?;

    // Parse JSON response
    let classifications: Vec<ClassificationEntry> = util::parse_json_response(&response)?;

    let mut result = Vec::new();
    for entry in classifications {
        let idx = entry.index;
        if idx < summaries.len() {
            // Normalize domain name to master and get stable slug
            let domain_def = domains::normalize(&entry.domain);
            result.push(ClassifiedConversation {
                summary: summaries[idx].clone(),
                domain: domain_def.name.to_string(),
                slug: domain_def.slug.to_string(),
                tags: entry.tags,
                confidence: entry.confidence,
            });
        }
    }

    Ok(result)
}

#[derive(serde::Deserialize)]
struct ClassificationEntry {
    index: usize,
    domain: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "default_confidence")]
    confidence: f64,
}

fn default_confidence() -> f64 {
    0.5
}

/// Group classified conversations by domain
pub fn group_by_domain(
    classified: &[ClassifiedConversation],
) -> std::collections::HashMap<String, Vec<&ClassifiedConversation>> {
    let mut groups = std::collections::HashMap::new();
    for c in classified {
        groups
            .entry(c.domain.clone())
            .or_insert_with(Vec::new)
            .push(c);
    }
    groups
}
