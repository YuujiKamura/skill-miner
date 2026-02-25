use crate::types::{Conversation, ConversationSummary, Role};
use std::collections::HashSet;

/// Compress a full conversation into a summary suitable for classification.
/// Extracts first user message, tool usage patterns, and key topics.
pub fn compress(conv: &Conversation) -> ConversationSummary {
    let first_message = conv
        .first_user_message()
        .unwrap_or("")
        .chars()
        .take(500)
        .collect::<String>();

    let tools_used: Vec<String> = {
        let mut seen = HashSet::new();
        conv.messages
            .iter()
            .flat_map(|m| m.tool_uses.iter())
            .filter(|t| seen.insert(t.name.clone()))
            .map(|t| t.name.clone())
            .collect()
    };

    let topics = extract_topics(conv);

    ConversationSummary {
        id: conv.id.clone(),
        source_path: conv.source_path.clone(),
        first_message,
        message_count: conv.message_count(),
        start_time: conv.start_time.clone(),
        cwd: conv.cwd.clone(),
        topics,
        tools_used,
    }
}

/// Compress multiple conversations
pub fn compress_all(conversations: &[Conversation]) -> Vec<ConversationSummary> {
    conversations.iter().map(compress).collect()
}

/// Extract key topics from conversation by looking at user messages
fn extract_topics(conv: &Conversation) -> Vec<String> {
    let mut topics = Vec::new();

    // Collect user messages (first 5 for efficiency)
    let user_messages: Vec<&str> = conv
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .take(5)
        .map(|m| m.content.as_str())
        .collect();

    // Extract file paths mentioned
    for msg in &user_messages {
        // Look for file extensions
        for ext in &[".rs", ".py", ".ts", ".xlsx", ".pdf", ".json", ".toml", ".md", ".dxf"] {
            if msg.contains(ext) {
                topics.push(format!("file:{}", ext));
            }
        }
    }

    // Extract project-related keywords
    let keywords = [
        ("舗装", "pavement"),
        ("写真", "photo"),
        ("PDF", "pdf"),
        ("施工", "construction"),
        ("スプレッド", "spreadsheet"),
        ("Excel", "excel"),
        ("Rust", "rust"),
        ("WASM", "wasm"),
        ("Git", "git"),
        ("テスト", "test"),
        ("区画線", "lane-marking"),
        ("横断", "cross-section"),
        ("品質", "quality"),
        ("出来形", "dekigata"),
        ("温度", "temperature"),
        ("工程", "schedule"),
        ("カルテ", "karte"),
        ("スキル", "skill"),
        ("Gemini", "gemini"),
        ("Claude", "claude"),
        ("DXF", "dxf"),
        ("レイアウト", "layout"),
    ];

    let all_text = user_messages.join(" ");
    for (jp, en) in &keywords {
        if all_text.contains(jp) {
            topics.push(en.to_string());
        }
    }

    topics.sort();
    topics.dedup();
    topics
}

/// Format summaries as a single text block for AI classification
pub fn format_for_classification(summaries: &[ConversationSummary]) -> String {
    let mut output = String::new();

    for (i, s) in summaries.iter().enumerate() {
        output.push_str(&format!(
            "[{}] id={} msgs={} cwd={} topics=[{}]\n  {}\n\n",
            i,
            &s.id[..8.min(s.id.len())],
            s.message_count,
            s.cwd.as_deref().unwrap_or("?"),
            s.topics.join(", "),
            truncate(&s.first_message, 200),
        ));
    }

    output
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}
