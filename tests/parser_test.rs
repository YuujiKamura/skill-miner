use skill_miner::parser;
use skill_miner::types::Role;
use std::path::Path;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sample_conversation.jsonl")
}

#[test]
fn parse_fixture_message_count() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    // 8 lines total:
    //   user1 (with system-reminder stripped) -> kept (has real text after strip)
    //   assistant1 (with tool_use) -> kept
    //   user2 -> kept
    //   assistant2 -> kept
    //   user3 -> kept
    //   assistant3 (with tool_use) -> kept
    //   meta message (isMeta=true) -> excluded
    //   file-history-snapshot -> excluded
    assert_eq!(conv.message_count(), 6);
}

#[test]
fn parse_fixture_roles_alternate() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    // Messages should alternate user/assistant
    for (i, msg) in conv.messages.iter().enumerate() {
        let expected = if i % 2 == 0 {
            Role::User
        } else {
            Role::Assistant
        };
        assert_eq!(msg.role, expected, "Message {} has wrong role", i);
    }
}

#[test]
fn parse_fixture_timestamps_present() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    assert_eq!(
        conv.start_time.as_deref(),
        Some("2026-01-15T10:00:00.000Z")
    );
    // end_time is the last timestamp encountered (including meta/snapshot lines)
    assert!(conv.end_time.is_some());
}

#[test]
fn parse_fixture_system_reminder_stripped() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    let first_user = conv.first_user_message().unwrap();
    assert!(
        !first_user.contains("system-reminder"),
        "system-reminder tag should be stripped, got: {}",
        first_user
    );
    assert!(
        first_user.contains("Fix the build error"),
        "Real user text should remain, got: {}",
        first_user
    );
}

#[test]
fn parse_fixture_meta_excluded() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    // The /clear meta message should not appear
    for msg in &conv.messages {
        assert!(
            !msg.content.contains("/clear"),
            "Meta message should be excluded"
        );
    }
}

#[test]
fn parse_fixture_file_history_snapshot_excluded() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    // file-history-snapshot entries should not create messages
    for msg in &conv.messages {
        assert!(
            !msg.content.contains("file-history-snapshot"),
            "file-history-snapshot should be excluded"
        );
    }
}

#[test]
fn parse_fixture_tool_uses_extracted() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    // assistant1 has Read tool, assistant3 has Bash tool
    let assistant_with_tools: Vec<_> = conv
        .messages
        .iter()
        .filter(|m| m.role == Role::Assistant && !m.tool_uses.is_empty())
        .collect();
    assert_eq!(assistant_with_tools.len(), 2, "Two assistants have tool uses");
    assert_eq!(assistant_with_tools[0].tool_uses[0].name, "Read");
    assert_eq!(assistant_with_tools[1].tool_uses[0].name, "Bash");
}

#[test]
fn parse_fixture_metadata() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    assert_eq!(conv.cwd.as_deref(), Some("/home/testuser/my-project"));
    assert_eq!(conv.git_branch.as_deref(), Some("main"));
}

#[test]
fn parse_fixture_id_from_filename() {
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    assert_eq!(conv.id, "sample_conversation");
}
