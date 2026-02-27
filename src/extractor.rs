use crate::error::SkillMinerError;
use crate::parser;
use crate::types::{ClassifiedConversation, Conversation, DomainCluster, KnowledgePattern, Role};
use crate::util;
use cli_ai_analyzer::{prompt, AnalyzeOptions};
use rayon::prelude::*;
use std::collections::HashMap;

/// Prompt template for extraction (loaded from file at compile time).
const EXTRACT_PROMPT: &str = include_str!("../prompts/extract.txt");

/// Remove `<system-reminder>...</system-reminder>` blocks from text.
fn strip_system_reminders(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find("<system-reminder>") {
        result.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find("</system-reminder>") {
            remaining = &remaining[start + end + "</system-reminder>".len()..];
        } else {
            // Unclosed tag - skip rest
            remaining = "";
            break;
        }
    }
    result.push_str(remaining);
    result
}

/// Extract knowledge patterns from a domain cluster.
/// When `conv_map` is provided, uses pre-parsed conversations to avoid re-parsing.
/// When `conv_map` is None (e.g. standalone `extract` command), falls back to parsing from source_path.
pub fn extract_patterns(
    domain: &str,
    conversations: &[&ClassifiedConversation],
    conv_map: Option<&HashMap<String, &Conversation>>,
    options: &AnalyzeOptions,
) -> Result<DomainCluster, SkillMinerError> {
    // Build context from full conversations (limited to avoid token overflow)
    let mut context_parts = Vec::new();

    for (i, conv) in conversations.iter().take(20).enumerate() {
        // Use pre-parsed conversation from map if available, otherwise parse from file
        let owned_conv;
        let full_conv = if let Some(map) = conv_map {
            if let Some(c) = map.get(&conv.summary.id) {
                *c
            } else {
                owned_conv = parser::parse_conversation(&conv.summary.source_path)?;
                &owned_conv
            }
        } else {
            owned_conv = parser::parse_conversation(&conv.summary.source_path)?;
            &owned_conv
        };

        // Extract user-assistant pairs (first 10 exchanges per conversation)
        let mut exchanges = Vec::new();
        let mut user_msg = None;

        for msg in full_conv.messages.iter().take(40) {
            match msg.role {
                Role::User => {
                    let cleaned = strip_system_reminders(&msg.content);
                    user_msg = Some(util::truncate(&cleaned, 2000));
                }
                Role::Assistant => {
                    if let Some(u) = user_msg.take() {
                        let cleaned_a = strip_system_reminders(&msg.content);
                        let a = util::truncate(&cleaned_a, 3000);
                        exchanges.push(format!("U: {}\nA: {}", u, a));
                    }
                }
            }
        }

        if !exchanges.is_empty() {
            let mut header = format!(
                "=== Conversation {} (id: {}) ===",
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

    let prompt_text = EXTRACT_PROMPT
        .replace("{domain}", domain)
        .replace("{context}", &context);

    let response = prompt(&prompt_text, options.clone())
        .map_err(|e| SkillMinerError::Ai(e.to_string()))?;
    let patterns: Vec<PatternEntry> = util::parse_json_response(&response)
        .map_err(|e| SkillMinerError::Parse(e.to_string()))?;

    let source_ids: Vec<String> = conversations.iter().map(|c| c.summary.id.clone()).collect();
    let knowledge_patterns: Vec<KnowledgePattern> = patterns
        .into_iter()
        .map(|p| p.into_knowledge_pattern(source_ids.clone()))
        .collect();

    Ok(DomainCluster {
        domain: domain.to_string(),
        conversations: conversations.iter().map(|c| (*c).clone()).collect(),
        patterns: knowledge_patterns,
    })
}

/// Extract patterns from all domains in parallel using rayon.
/// When `conv_map` is provided, avoids re-parsing conversations from disk.
/// `max_parallel` controls the maximum number of concurrent AI calls.
/// Returns (clusters, extract_call_count, failed_domain_names).
pub fn extract_all_parallel(
    groups: &HashMap<String, Vec<&ClassifiedConversation>>,
    conv_map: Option<&HashMap<String, &Conversation>>,
    options: &AnalyzeOptions,
    max_parallel: usize,
) -> Result<(Vec<DomainCluster>, usize, Vec<String>), SkillMinerError> {
    const MIN_CONVERSATIONS: usize = 1;
    let entries: Vec<_> = groups
        .iter()
        .filter(|(domain, convs)| {
            if convs.len() < MIN_CONVERSATIONS {
                eprintln!(
                    "  [SKIP] {} — {} conversations (min {})",
                    domain,
                    convs.len(),
                    MIN_CONVERSATIONS
                );
                false
            } else {
                true
            }
        })
        .collect();

    let num_threads = max_parallel.min(entries.len()).max(1);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .map_err(|e| SkillMinerError::Config(format!("rayon pool: {}", e)))?;

    let results: Vec<(String, Result<DomainCluster, SkillMinerError>)> = pool.install(|| {
        entries
            .par_iter()
            .map(|(domain, convs)| {
                eprintln!("  {} ({} conversations)...", domain, convs.len());
                let r = extract_patterns(domain, convs, conv_map, options);
                (domain.to_string(), r)
            })
            .collect()
    });

    let mut clusters = Vec::new();
    let mut failed_domains = Vec::new();
    for (domain, result) in results {
        match result {
            Ok(cluster) => clusters.push(cluster),
            Err(e) => {
                eprintln!("  [SKIP] {} extract failed: {} — continuing", domain, e);
                failed_domains.push(domain);
            }
        }
    }

    let call_count = clusters.len() + failed_domains.len();

    // Sort by domain name for deterministic output
    clusters.sort_by(|a, b| a.domain.cmp(&b.domain));

    Ok((clusters, call_count, failed_domains))
}

#[derive(serde::Deserialize)]
struct PatternEntry {
    #[serde(default)]
    skill_slug: Option<String>,
    title: String,
    description: String,
    #[serde(default)]
    steps: Vec<String>,
    #[serde(default)]
    code_examples: Vec<String>,
    /// Legacy: old prompt returns frequency. New prompt returns discussed: true.
    /// Accept either format.
    #[serde(default = "default_freq")]
    frequency: usize,
    #[serde(default)]
    discussed: bool,
}

impl PatternEntry {
    fn into_knowledge_pattern(self, source_ids: Vec<String>) -> KnowledgePattern {
        KnowledgePattern {
            title: self.title,
            description: self.description,
            steps: self.steps,
            code_examples: self.code_examples,
            source_ids,
            frequency: self.frequency,
            skill_slug: self.skill_slug,
        }
    }
}

fn default_freq() -> usize {
    1
}

