use crate::compressor;
use crate::domains;
use crate::error::SkillMinerError;
use crate::types::{ClassifiedConversation, ConversationSummary};
use crate::util;
use cli_ai_analyzer::{prompt, AnalyzeOptions};

/// Classify conversation summaries into domain clusters using AI
pub fn classify(
    summaries: &[ConversationSummary],
    options: &AnalyzeOptions,
) -> Result<Vec<ClassifiedConversation>, SkillMinerError> {
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

/// Prompt template for classification (loaded from file at compile time).
const CLASSIFY_PROMPT: &str = include_str!("../prompts/classify.txt");

fn classify_batch(
    summaries: &[ConversationSummary],
    formatted_text: &str,
    options: &AnalyzeOptions,
) -> Result<Vec<ClassifiedConversation>, SkillMinerError> {
    let domain_list = domains::prompt_domain_list();

    let prompt_text = CLASSIFY_PROMPT
        .replace("{domain_list}", &domain_list)
        .replace("{formatted_text}", formatted_text);

    let response = prompt(&prompt_text, options.clone())
        .map_err(|e| SkillMinerError::Ai(e.to_string()))?;

    // Parse JSON response
    let classifications: Vec<ClassificationEntry> = util::parse_json_response(&response)
        .map_err(|e| SkillMinerError::Parse(e.to_string()))?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_summary(id: &str) -> ConversationSummary {
        ConversationSummary {
            id: id.to_string(),
            source_path: PathBuf::from("/tmp/dummy.jsonl"),
            first_message: "dummy message".to_string(),
            message_count: 4,
            start_time: None,
            cwd: Some("/tmp".to_string()),
            topics: vec![],
            tools_used: vec![],
            files_touched: vec![],
            commands_used: vec![],
        }
    }

    fn make_classified(id: &str, domain: &str) -> ClassifiedConversation {
        ClassifiedConversation {
            summary: make_summary(id),
            domain: domain.to_string(),
            slug: String::new(),
            tags: vec![],
            confidence: 0.9,
        }
    }

    #[test]
    fn group_by_domain_empty() {
        let classified: Vec<ClassifiedConversation> = vec![];
        let groups = group_by_domain(&classified);
        assert!(groups.is_empty());
    }

    #[test]
    fn group_by_domain_single() {
        let classified = vec![make_classified("conv1", "Rust開発")];
        let groups = group_by_domain(&classified);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups["Rust開発"].len(), 1);
        assert_eq!(groups["Rust開発"][0].summary.id, "conv1");
    }

    #[test]
    fn group_by_domain_multiple_domains() {
        let classified = vec![
            make_classified("conv1", "Rust開発"),
            make_classified("conv2", "AI連携"),
            make_classified("conv3", "Rust開発"),
            make_classified("conv4", "PDF操作"),
            make_classified("conv5", "AI連携"),
        ];
        let groups = group_by_domain(&classified);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups["Rust開発"].len(), 2);
        assert_eq!(groups["AI連携"].len(), 2);
        assert_eq!(groups["PDF操作"].len(), 1);
    }

    #[test]
    fn group_by_domain_all_same() {
        let classified = vec![
            make_classified("c1", "その他"),
            make_classified("c2", "その他"),
        ];
        let groups = group_by_domain(&classified);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups["その他"].len(), 2);
    }

    #[test]
    fn classification_entry_deserialize_defaults() {
        let json = r#"{"index": 0, "domain": "Rust開発"}"#;
        let entry: ClassificationEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.index, 0);
        assert_eq!(entry.domain, "Rust開発");
        assert!(entry.tags.is_empty());
        assert!((entry.confidence - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn classification_entry_deserialize_full() {
        let json = r#"{"index": 2, "domain": "PDF操作", "tags": ["gen", "merge"], "confidence": 0.88}"#;
        let entry: ClassificationEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.index, 2);
        assert_eq!(entry.tags, vec!["gen", "merge"]);
        assert!((entry.confidence - 0.88).abs() < f64::EPSILON);
    }
}
