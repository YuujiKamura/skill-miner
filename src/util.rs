use anyhow::Result;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

/// Truncate a string at a safe char boundary, appending "..." if truncated.
pub fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let end: String = s.chars().take(max_chars).collect();
        format!("{}...", end)
    }
}

/// Sanitize AI response: remove control characters that break JSON parsing.
/// Inside JSON string values, raw newlines/tabs are invalid — replace all control chars.
/// Then restore structural newlines between JSON elements.
pub fn sanitize_json(s: &str) -> String {
    // Phase 1: Replace ALL control characters with spaces
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    // Phase 2: Collapse multiple spaces into single space (optional, keeps output clean)
    cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse a JSON array from AI response, handling markdown code fences and control characters
pub fn parse_json_response<T: DeserializeOwned>(response: &str) -> Result<Vec<T>> {
    let sanitized = sanitize_json(response);
    let trimmed = sanitized.trim();

    // Extract JSON array from response
    let json_str = if let Some(start) = trimmed.find('[') {
        let end = trimmed.rfind(']').map(|i| i + 1).unwrap_or(trimmed.len());
        &trimmed[start..end]
    } else {
        trimmed
    };

    serde_json::from_str(json_str).map_err(|e| {
        let preview: String = response.chars().take(200).collect();
        anyhow::anyhow!("Failed to parse JSON array: {}\nResponse: {}", e, preview)
    })
}

/// Normalize a path to forward slashes for consistent display.
pub fn normalize_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Get the user's home directory (cross-platform).
pub fn home_dir() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct TestEntry {
        name: String,
        value: i32,
    }

    #[test]
    fn test_truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_exact_length() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_long_string() {
        assert_eq!(truncate("hello world", 5), "hello...");
    }

    #[test]
    fn test_truncate_multibyte() {
        // Japanese characters are multi-byte but each is 1 char
        assert_eq!(truncate("あいうえお", 3), "あいう...");
    }

    #[test]
    fn test_sanitize_json_removes_control_chars() {
        let input = "hello\x00world\x01test\n\r\t";
        let result = sanitize_json(input);
        assert_eq!(result, "hello world test");
    }

    #[test]
    fn test_parse_json_response_plain_array() {
        let input = r#"[{"name": "a", "value": 1}, {"name": "b", "value": 2}]"#;
        let result: Vec<TestEntry> = parse_json_response(input).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "a");
        assert_eq!(result[1].value, 2);
    }

    #[test]
    fn test_parse_json_response_with_markdown_fence() {
        let input = "```json\n[{\"name\": \"x\", \"value\": 42}]\n```";
        let result: Vec<TestEntry> = parse_json_response(input).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "x");
    }

    #[test]
    fn test_parse_json_response_with_surrounding_text() {
        let input = "Here are the results:\n[{\"name\": \"test\", \"value\": 99}]\nDone.";
        let result: Vec<TestEntry> = parse_json_response(input).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].value, 99);
    }

    #[test]
    fn test_parse_json_response_invalid_json() {
        let input = "not json at all";
        let result: Result<Vec<TestEntry>> = parse_json_response(input);
        assert!(result.is_err());
    }
}
