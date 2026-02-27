use skill_miner::classifier;
use skill_miner::types::{ClassifiedConversation, ConversationSummary};
use skill_miner::util;
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

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

// --- group_by_domain integration tests ---

#[test]
fn group_by_domain_empty_list() {
    let classified: Vec<ClassifiedConversation> = vec![];
    let groups = classifier::group_by_domain(&classified);
    assert!(groups.is_empty());
}

#[test]
fn group_by_domain_single_item() {
    let classified = vec![make_classified("conv1", "CLI & Tooling")];
    let groups = classifier::group_by_domain(&classified);
    assert_eq!(groups.len(), 1);
    assert!(groups.contains_key("CLI & Tooling"));
    assert_eq!(groups["CLI & Tooling"].len(), 1);
}

#[test]
fn group_by_domain_multiple_domains() {
    let classified = vec![
        make_classified("c1", "CLI & Tooling"),
        make_classified("c2", "AI & Machine Learning"),
        make_classified("c3", "CLI & Tooling"),
        make_classified("c4", "Database & Storage"),
    ];
    let groups = classifier::group_by_domain(&classified);
    assert_eq!(groups.len(), 3);
    assert_eq!(groups["CLI & Tooling"].len(), 2);
    assert_eq!(groups["AI & Machine Learning"].len(), 1);
    assert_eq!(groups["Database & Storage"].len(), 1);
}

#[test]
fn group_by_domain_preserves_references() {
    let classified = vec![
        make_classified("aaa", "Testing & QA"),
        make_classified("bbb", "Testing & QA"),
    ];
    let groups = classifier::group_by_domain(&classified);
    let refs = &groups["Testing & QA"];
    assert_eq!(refs[0].summary.id, "aaa");
    assert_eq!(refs[1].summary.id, "bbb");
}

// --- JSON response parsing edge cases ---

#[test]
fn parse_classify_fixture() {
    let fixture = std::fs::read_to_string(fixture_dir().join("classify_response.json")).unwrap();

    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Entry {
        index: usize,
        domain: String,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        confidence: f64,
    }

    let entries: Vec<Entry> = util::parse_json_response(&fixture).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].domain, "Web Development");
    assert_eq!(entries[1].domain, "AI & Machine Learning");
    assert_eq!(entries[2].domain, "Database & Storage");
    assert!((entries[0].confidence - 0.95).abs() < f64::EPSILON);
}

#[test]
fn parse_json_with_code_fence() {
    let input = "```json\n[{\"index\": 0, \"domain\": \"Web Development\"}]\n```";

    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Entry {
        index: usize,
        domain: String,
    }

    let entries: Vec<Entry> = util::parse_json_response(input).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].domain, "Web Development");
}

#[test]
fn parse_json_with_control_chars() {
    // Simulate AI response with control characters embedded
    let input = "[{\"index\": 0, \"domain\": \"Web\x01Development\"}]";

    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Entry {
        index: usize,
        domain: String,
    }

    let entries: Vec<Entry> = util::parse_json_response(input).unwrap();
    assert_eq!(entries.len(), 1);
    // Control char replaced with space
    assert_eq!(entries[0].domain, "Web Development");
}

#[test]
fn parse_json_with_surrounding_text() {
    let input = "Here are the results:\n[{\"index\": 0, \"domain\": \"Database & Storage\"}]\nDone.";

    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Entry {
        index: usize,
        domain: String,
    }

    let entries: Vec<Entry> = util::parse_json_response(input).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].domain, "Database & Storage");
}

#[test]
fn parse_json_empty_array() {
    let input = "[]";

    #[derive(serde::Deserialize, Debug)]
    struct Entry {
        _index: usize,
    }

    let entries: Vec<Entry> = util::parse_json_response(input).unwrap();
    assert!(entries.is_empty());
}

#[test]
fn parse_json_no_array_returns_error() {
    let input = "This is not JSON at all";

    #[derive(serde::Deserialize)]
    struct Entry {
        _index: usize,
    }

    let result: Result<Vec<Entry>, _> = util::parse_json_response(input);
    assert!(result.is_err());
}
