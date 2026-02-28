use crate::error::SkillMinerError;
use crate::parser;
use crate::types::{ClassifiedConversation, Conversation, DomainCluster, KnowledgePattern, Role};
use crate::util;
use cli_ai_analyzer::{prompt, AnalyzeOptions};
use rayon::prelude::*;
use std::collections::HashMap;

/// Prompt template for extraction (loaded from file at compile time).
const EXTRACT_PROMPT: &str = include_str!("../prompts/extract.txt");

/// Prompt template for pre-summarization (loaded from file at compile time).
const SUMMARIZE_PROMPT: &str = include_str!("../prompts/summarize.txt");

/// Maximum number of conversations to include in context.
const MAX_CONVERSATIONS: usize = 20;

/// Maximum number of messages to scan per conversation for user-assistant exchanges.
const MAX_MESSAGES_PER_CONV: usize = 40;

/// Truncation length (chars) for user messages in context.
const USER_MSG_TRUNCATE_LEN: usize = 2000;

/// Truncation length (chars) for assistant messages in context.
const ASSISTANT_MSG_TRUNCATE_LEN: usize = 3000;

/// Maximum number of file paths to include in conversation header metadata.
const MAX_FILES_IN_HEADER: usize = 10;

/// Maximum number of commands to include in conversation header metadata.
const MAX_CMDS_IN_HEADER: usize = 5;

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

/// Build a context string from conversations for the extraction prompt.
///
/// Iterates over conversations, extracts user-assistant exchanges, and formats
/// them with header metadata (files touched, commands used) into a single string.
fn build_extraction_context(
    conversations: &[&ClassifiedConversation],
    conv_map: Option<&HashMap<String, &Conversation>>,
) -> Result<String, SkillMinerError> {
    let mut context_parts = Vec::new();

    for (i, conv) in conversations.iter().take(MAX_CONVERSATIONS).enumerate() {
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

        for msg in full_conv.messages.iter().take(MAX_MESSAGES_PER_CONV) {
            match msg.role {
                Role::User => {
                    let cleaned = strip_system_reminders(&msg.content);
                    user_msg = Some(util::truncate(&cleaned, USER_MSG_TRUNCATE_LEN));
                }
                Role::Assistant => {
                    if let Some(u) = user_msg.take() {
                        let cleaned_a = strip_system_reminders(&msg.content);
                        let a = util::truncate(&cleaned_a, ASSISTANT_MSG_TRUNCATE_LEN);
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
                let files: Vec<_> = conv.summary.files_touched.iter().take(MAX_FILES_IN_HEADER).map(|f| f.as_str()).collect();
                header.push_str(&format!("\nfiles: [{}]", files.join(", ")));
            }
            if !conv.summary.commands_used.is_empty() {
                let cmds: Vec<_> = conv.summary.commands_used.iter().take(MAX_CMDS_IN_HEADER).map(|c| c.as_str()).collect();
                header.push_str(&format!("\ncmds: [{}]", cmds.join(", ")));
            }
            context_parts.push(format!(
                "{}\n{}",
                header,
                exchanges.join("\n---\n")
            ));
        }
    }

    Ok(context_parts.join("\n\n"))
}

/// Extract knowledge patterns from a domain cluster.
/// When `conv_map` is provided, uses pre-parsed conversations to avoid re-parsing.
/// When `conv_map` is None (e.g. standalone `extract` command), falls back to parsing from source_path.
pub fn extract_patterns(
    domain: &str,
    conversations: &[&ClassifiedConversation],
    conv_map: Option<&HashMap<String, &Conversation>>,
    options: &AnalyzeOptions,
    summarize_options: Option<&AnalyzeOptions>,
) -> Result<DomainCluster, SkillMinerError> {
    let raw_context = build_extraction_context(conversations, conv_map)?;

    // Pre-summarize with a separate model if configured
    let context = if let Some(sum_opts) = summarize_options {
        let sum_prompt = SUMMARIZE_PROMPT
            .replace("{domain}", domain)
            .replace("{context}", &raw_context);
        eprintln!("    [summarize] {} with {}...", domain, sum_opts.model);
        match prompt(&sum_prompt, sum_opts.clone()) {
            Ok(summary) => summary,
            Err(e) => {
                eprintln!("    [summarize] failed: {} — falling back to raw context", e);
                raw_context
            }
        }
    } else {
        raw_context
    };

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
    summarize_options: Option<&AnalyzeOptions>,
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
                let r = extract_patterns(domain, convs, conv_map, options, summarize_options);
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
    #[serde(default = "default_freq")]
    frequency: usize,
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

