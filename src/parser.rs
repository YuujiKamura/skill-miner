use crate::error::SkillMinerError;
use crate::types::{Conversation, Message, Role, SkillInvocation, ToolUse};
use crate::util;
use chrono::{DateTime, Duration, Utc};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Parse a single conversation JSONL file into a Conversation struct
pub fn parse_conversation(path: &Path) -> Result<Conversation, SkillMinerError> {
    let file = File::open(path).map_err(|e| {
        SkillMinerError::Parse(format!("opening {}: {}", path.display(), e))
    })?;
    let reader = BufReader::new(file);

    let id = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut messages = Vec::new();
    let mut start_time: Option<DateTime<Utc>> = None;
    let mut end_time: Option<DateTime<Utc>> = None;
    let mut cwd = None;
    let mut git_branch = None;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let entry: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Skip non-message entries
        let entry_type = entry.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if entry_type == "file-history-snapshot" {
            continue;
        }

        // Extract metadata from first entry
        if cwd.is_none() {
            cwd = entry.get("cwd").and_then(|v| v.as_str()).map(String::from);
        }
        if git_branch.is_none() {
            git_branch = entry
                .get("gitBranch")
                .and_then(|v| v.as_str())
                .map(String::from);
        }

        let ts_parsed: Option<DateTime<Utc>> = entry
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<DateTime<Utc>>().ok());

        if start_time.is_none() && ts_parsed.is_some() {
            start_time = ts_parsed;
        }
        if ts_parsed.is_some() {
            end_time = ts_parsed;
        }

        // Skip meta messages (commands, system)
        if entry.get("isMeta").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }

        let message = match entry.get("message") {
            Some(msg) => msg,
            None => continue,
        };

        let role_str = message.get("role").and_then(|r| r.as_str()).unwrap_or("");
        let role = match role_str {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            _ => continue,
        };

        let (content, tool_uses) = extract_content(message);

        // Skip empty or system-only content
        if content.trim().is_empty() && tool_uses.is_empty() {
            continue;
        }

        // Skip system-reminder-only user messages
        if role == Role::User && is_system_only(&content) {
            continue;
        }

        messages.push(Message {
            role,
            content,
            timestamp: ts_parsed,
            tool_uses,
        });
    }

    Ok(Conversation {
        id,
        source_path: path.to_path_buf(),
        messages,
        start_time,
        end_time,
        cwd,
        git_branch,
    })
}

/// Extract text content and tool uses from a message value
fn extract_content(message: &serde_json::Value) -> (String, Vec<ToolUse>) {
    let content = message.get("content");
    let mut text_parts = Vec::new();
    let mut tool_uses = Vec::new();

    match content {
        Some(serde_json::Value::String(s)) => {
            text_parts.push(strip_tags(s));
        }
        Some(serde_json::Value::Array(blocks)) => {
            for block in blocks {
                match block.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            text_parts.push(strip_tags(text));
                        }
                    }
                    Some("tool_use") => {
                        let name = block
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let input_val = block.get("input");
                        let input = input_val.map(|i| {
                            let s = i.to_string();
                            util::truncate(&s, 200)
                        }).unwrap_or_default();

                        // Extract file_path for Edit/Read/Write tools
                        let file_path = match name.as_str() {
                            "Edit" | "Read" | "Write" => input_val
                                .and_then(|i| i.get("file_path"))
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            _ => None,
                        };

                        // Extract command for Bash tool
                        let command = if name == "Bash" {
                            input_val
                                .and_then(|i| i.get("command"))
                                .and_then(|v| v.as_str())
                                .map(|s| util::truncate(s, 100))
                        } else {
                            None
                        };

                        tool_uses.push(ToolUse {
                            name,
                            input_summary: input,
                            file_path,
                            command,
                        });
                    }
                    Some("tool_result") => {
                        // Skip tool results - they're execution output, not knowledge
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    (text_parts.join("\n"), tool_uses)
}

/// Remove XML-like system tags from content
fn strip_tags(s: &str) -> String {
    let s = remove_tag_block(s, "system-reminder");
    let s = remove_tag_block(&s, "local-command-caveat");
    let s = remove_tag_block(&s, "command-name");
    let s = remove_tag_block(&s, "command-message");
    let s = remove_tag_block(&s, "command-args");

    s.trim().to_string()
}

fn remove_tag_block(s: &str, tag: &str) -> String {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let mut result = s.to_string();

    while let Some(start) = result.find(&open) {
        if let Some(end) = result[start..].find(&close) {
            let end_pos = start + end + close.len();
            result = format!("{}{}", &result[..start], &result[end_pos..]);
        } else {
            break;
        }
    }

    result
}

/// Check if content is only system tags with no real user input
fn is_system_only(content: &str) -> bool {
    let stripped = content.trim();
    stripped.is_empty()
        || stripped.starts_with("<local-command-caveat>")
        || stripped.starts_with("<command-name>")
}

/// Discover all conversation JSONL files in a project directory
pub fn discover_conversations(projects_dir: &Path) -> Result<Vec<PathBuf>, SkillMinerError> {
    let mut paths = Vec::new();

    if !projects_dir.exists() {
        return Err(SkillMinerError::Config(format!(
            "Projects directory not found: {}",
            projects_dir.display()
        )));
    }

    // Walk project subdirectories
    for entry in std::fs::read_dir(projects_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            // Look for .jsonl files inside project dirs
            for sub_entry in std::fs::read_dir(&path)? {
                let sub_entry = sub_entry?;
                let sub_path = sub_entry.path();
                if sub_path.extension().map(|e| e == "jsonl").unwrap_or(false) && sub_path.is_file()
                {
                    paths.push(sub_path);
                }
            }
        }
    }

    // Sort by modification time (newest first)
    paths.sort_by(|a, b| {
        let a_time = std::fs::metadata(a)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let b_time = std::fs::metadata(b)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        b_time.cmp(&a_time)
    });

    Ok(paths)
}

/// Parse all conversations, filtering by minimum message count and days_back.
/// days_back=0 means no time filter (all conversations included).
pub fn parse_all(projects_dir: &Path, min_messages: usize, days_back: u32) -> Result<Vec<Conversation>, SkillMinerError> {
    let paths = discover_conversations(projects_dir)?;
    let cutoff = if days_back > 0 {
        Some(Utc::now() - Duration::days(days_back as i64))
    } else {
        None
    };
    let mut conversations = Vec::new();

    for path in &paths {
        match parse_conversation(path) {
            Ok(conv) if conv.message_count() >= min_messages => {
                // Apply days_back filter: skip conversations whose start_time is before cutoff.
                // Conversations with no timestamp are always included.
                if let Some(ref cutoff) = cutoff {
                    if let Some(dt) = conv.start_time {
                        if dt < *cutoff {
                            continue;
                        }
                    }
                }
                conversations.push(conv);
            }
            Ok(_) => {} // too short, skip
            Err(e) => {
                eprintln!("Warning: skipping {}: {}", path.display(), e);
            }
        }
    }

    Ok(conversations)
}

/// Parse conversations within a specific time window [start, end).
/// Conversations with no timestamp are excluded from windowed parsing.
pub fn parse_window(
    projects_dir: &Path,
    min_messages: usize,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<Conversation>, SkillMinerError> {
    let paths = discover_conversations(projects_dir)?;
    let mut conversations = Vec::new();

    for path in &paths {
        match parse_conversation(path) {
            Ok(conv) if conv.message_count() >= min_messages => {
                if let Some(dt) = conv.start_time {
                    if dt >= start && dt < end {
                        conversations.push(conv);
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("Warning: skipping {}: {}", path.display(), e);
            }
        }
    }

    Ok(conversations)
}

/// Extract all Skill tool invocations from parsed conversations.
/// Determines was_productive by checking if the next assistant message after
/// the Skill invocation contains any tool_use (meaning the skill led to action).
pub fn extract_skill_invocations(conversations: &[Conversation]) -> Vec<SkillInvocation> {
    let mut invocations = Vec::new();

    for conv in conversations {
        for (i, msg) in conv.messages.iter().enumerate() {
            if msg.role != Role::Assistant {
                continue;
            }

            for tool_use in &msg.tool_uses {
                if tool_use.name != "Skill" {
                    continue;
                }

                let skill_name = extract_skill_name(&tool_use.input_summary);
                if skill_name.is_empty() {
                    continue;
                }

                // Check if the next assistant message has tool_uses
                let was_productive = conv.messages[i + 1..]
                    .iter()
                    .find(|m| m.role == Role::Assistant)
                    .map(|m| !m.tool_uses.is_empty())
                    .unwrap_or(false);

                // Find the most recent user message before this assistant message
                let trigger_context = conv.messages[..i]
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::User && !m.content.trim().is_empty())
                    .map(|m| crate::util::truncate(&m.content, 200));

                invocations.push(SkillInvocation {
                    skill_name,
                    conversation_id: conv.id.clone(),
                    timestamp: msg.timestamp,
                    was_productive,
                    trigger_context,
                });
            }
        }
    }

    invocations
}

/// Extract skill name from Skill tool input_summary.
/// Tries JSON parsing first, falls back to string pattern matching.
fn extract_skill_name(input_summary: &str) -> String {
    // Try JSON parse first
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(input_summary) {
        if let Some(name) = val.get("skill").and_then(|v| v.as_str()) {
            return name.to_string();
        }
    }

    // Fallback: find "skill":"..." or "skill": "..." pattern
    for pattern in &["\"skill\":\"", "\"skill\": \""] {
        if let Some(start) = input_summary.find(pattern) {
            let after = &input_summary[start + pattern.len()..];
            if let Some(end) = after.find('"') {
                return after[..end].to_string();
            }
        }
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_tags() {
        let input = "<system-reminder>foo</system-reminder>hello world";
        assert_eq!(strip_tags(input), "hello world");
    }

    #[test]
    fn test_is_system_only() {
        assert!(is_system_only("<local-command-caveat>some caveat</local-command-caveat>"));
        assert!(!is_system_only("actual user message"));
    }

    #[test]
    fn test_remove_tag_block() {
        let s = "before<system-reminder>hidden</system-reminder>after";
        assert_eq!(remove_tag_block(s, "system-reminder"), "beforeafter");
    }

    #[test]
    fn test_extract_skill_invocations() {
        use crate::types::*;
        let conversations = vec![Conversation {
            id: "conv1".to_string(),
            source_path: PathBuf::from("test.jsonl"),
            messages: vec![
                Message {
                    role: Role::User,
                    content: "organize the photos".to_string(),
                    timestamp: None,
                    tool_uses: vec![],
                },
                Message {
                    role: Role::Assistant,
                    content: String::new(),
                    timestamp: None,
                    tool_uses: vec![ToolUse {
                        name: "Skill".to_string(),
                        input_summary: r#"{"skill":"my-skill","args":""}"#.to_string(),
                        file_path: None,
                        command: None,
                    }],
                },
                Message {
                    role: Role::Assistant,
                    content: "doing work".to_string(),
                    timestamp: None,
                    tool_uses: vec![ToolUse {
                        name: "Edit".to_string(),
                        input_summary: "editing file".to_string(),
                        file_path: Some("test.rs".to_string()),
                        command: None,
                    }],
                },
            ],
            start_time: None,
            end_time: None,
            cwd: None,
            git_branch: None,
        }];
        let invocations = extract_skill_invocations(&conversations);
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].skill_name, "my-skill");
        assert!(invocations[0].was_productive);
        assert_eq!(invocations[0].trigger_context, Some("organize the photos".to_string()));
    }

    #[test]
    fn test_extract_skill_invocations_not_productive() {
        use crate::types::*;
        let conversations = vec![Conversation {
            id: "conv2".to_string(),
            source_path: PathBuf::from("test.jsonl"),
            messages: vec![
                Message {
                    role: Role::User,
                    content: "run the skill".to_string(),
                    timestamp: None,
                    tool_uses: vec![],
                },
                Message {
                    role: Role::Assistant,
                    content: String::new(),
                    timestamp: None,
                    tool_uses: vec![ToolUse {
                        name: "Skill".to_string(),
                        input_summary: r#"{"skill":"lonely-skill"}"#.to_string(),
                        file_path: None,
                        command: None,
                    }],
                },
                // No follow-up assistant message with tools
                Message {
                    role: Role::User,
                    content: "thanks".to_string(),
                    timestamp: None,
                    tool_uses: vec![],
                },
            ],
            start_time: None,
            end_time: None,
            cwd: None,
            git_branch: None,
        }];
        let invocations = extract_skill_invocations(&conversations);
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].skill_name, "lonely-skill");
        assert!(!invocations[0].was_productive);
        assert_eq!(invocations[0].trigger_context, Some("run the skill".to_string()));
    }

    #[test]
    fn test_trigger_context_truncated() {
        use crate::types::*;
        // Test that trigger_context is truncated at 200 chars
        let long_message = "a".repeat(300);
        let conversations = vec![Conversation {
            id: "conv3".to_string(),
            source_path: PathBuf::from("test.jsonl"),
            messages: vec![
                Message {
                    role: Role::User,
                    content: long_message.clone(),
                    timestamp: None,
                    tool_uses: vec![],
                },
                Message {
                    role: Role::Assistant,
                    content: String::new(),
                    timestamp: None,
                    tool_uses: vec![ToolUse {
                        name: "Skill".to_string(),
                        input_summary: r#"{"skill":"long-trigger"}"#.to_string(),
                        file_path: None,
                        command: None,
                    }],
                },
            ],
            start_time: None,
            end_time: None,
            cwd: None,
            git_branch: None,
        }];
        let invocations = extract_skill_invocations(&conversations);
        assert_eq!(invocations.len(), 1);
        let ctx = invocations[0].trigger_context.as_ref().unwrap();
        assert!(ctx.chars().count() <= 203); // 200 + "..."
    }

    #[test]
    fn test_parse_window_filters_by_range() {
        // Create a temp dir with a project subdir and a JSONL conversation
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("test-project");
        std::fs::create_dir_all(&project_dir).unwrap();

        // Create a JSONL file with a timestamp from 2 hours ago
        let two_hours_ago = Utc::now() - Duration::hours(2);
        let ts = two_hours_ago.to_rfc3339();
        let line1 = format!(
            r#"{{"timestamp":"{}","message":{{"role":"user","content":"hello"}},"cwd":"/tmp"}}"#,
            ts
        );
        let line2 = format!(
            r#"{{"timestamp":"{}","message":{{"role":"assistant","content":"world"}}}}"#,
            ts
        );
        let line3 = format!(
            r#"{{"timestamp":"{}","message":{{"role":"user","content":"more"}}}}"#,
            ts
        );
        let line4 = format!(
            r#"{{"timestamp":"{}","message":{{"role":"assistant","content":"stuff"}}}}"#,
            ts
        );
        let jsonl_content = format!("{}\n{}\n{}\n{}", line1, line2, line3, line4);
        std::fs::write(project_dir.join("conv-test.jsonl"), &jsonl_content).unwrap();

        // Window that includes 2h ago (0h-4h ago): should find it
        let now = Utc::now();
        let result = parse_window(dir.path(), 2, now - Duration::hours(4), now).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "conv-test");

        // Window that doesn't include 2h ago (5h-10h ago): should not find it
        let result = parse_window(
            dir.path(),
            2,
            now - Duration::hours(10),
            now - Duration::hours(5),
        )
        .unwrap();
        assert_eq!(result.len(), 0);
    }
}
