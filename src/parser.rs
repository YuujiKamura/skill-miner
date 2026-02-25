use crate::error::SkillMinerError;
use crate::types::{Conversation, Message, Role, ToolUse};
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
}
