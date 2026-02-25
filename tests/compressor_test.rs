use skill_miner::compressor;
use skill_miner::parser;
use std::path::Path;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sample_conversation.jsonl")
}

#[test]
fn compress_extracts_first_message() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    let summary = compressor::compress(&conv);
    assert!(
        summary.first_message.contains("Fix the build error"),
        "first_message should contain user's first text, got: {}",
        summary.first_message
    );
}

#[test]
fn compress_extracts_tools_used() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    let summary = compressor::compress(&conv);
    assert!(
        summary.tools_used.contains(&"Read".to_string()),
        "tools_used should contain Read, got: {:?}",
        summary.tools_used
    );
    assert!(
        summary.tools_used.contains(&"Bash".to_string()),
        "tools_used should contain Bash, got: {:?}",
        summary.tools_used
    );
}

#[test]
fn compress_tools_deduplicated() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    let summary = compressor::compress(&conv);
    // Each tool name should appear only once
    let mut seen = std::collections::HashSet::new();
    for tool in &summary.tools_used {
        assert!(seen.insert(tool), "Duplicate tool: {}", tool);
    }
}

#[test]
fn compress_extracts_topics() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    let summary = compressor::compress(&conv);
    // User messages mention ".rs" and "Rust" (via cargo test), "test"
    assert!(
        summary.topics.contains(&"file:.rs".to_string()),
        "topics should contain file:.rs, got: {:?}",
        summary.topics
    );
}

#[test]
fn compress_preserves_metadata() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    let summary = compressor::compress(&conv);
    assert_eq!(summary.id, "sample_conversation");
    assert_eq!(summary.message_count, 6);
    assert_eq!(
        summary.start_time.as_deref(),
        Some("2026-01-15T10:00:00.000Z")
    );
    assert_eq!(summary.cwd.as_deref(), Some("/home/testuser/my-project"));
}

#[test]
fn format_for_classification_output() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    let summaries = compressor::compress_all(&[conv]);
    let output = compressor::format_for_classification(&summaries);

    // Should contain index, id prefix, message count, cwd, and topics
    assert!(output.contains("[0]"), "Should contain index [0]");
    assert!(
        output.contains("id=sample_c"),
        "Should contain truncated id, got: {}",
        output
    );
    assert!(output.contains("msgs=6"), "Should contain message count");
    assert!(
        output.contains("/home/testuser/my-project"),
        "Should contain cwd"
    );
    assert!(
        output.contains("Fix the build error"),
        "Should contain first message text"
    );
}

#[test]
fn compress_all_batch() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    let conv2 = conv.clone();
    let summaries = compressor::compress_all(&[conv, conv2]);
    assert_eq!(summaries.len(), 2);
}
