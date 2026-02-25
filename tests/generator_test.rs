use skill_miner::generator;
use skill_miner::types::{DomainCluster, KnowledgePattern, SkillDraft};
use std::path::Path;

fn make_test_cluster() -> DomainCluster {
    DomainCluster {
        domain: "Rust開発".to_string(),
        conversations: vec![],
        patterns: vec![
            KnowledgePattern {
                title: "cargo checkでビルド確認".to_string(),
                description: "変更後にcargo checkで型エラーがないか確認する".to_string(),
                steps: vec![
                    "コードを変更する".to_string(),
                    "cargo checkを実行する".to_string(),
                    "エラーがあれば修正する".to_string(),
                ],
                source_ids: vec!["abc12345".to_string(), "def67890".to_string()],
                frequency: 5,
            },
            KnowledgePattern {
                title: "テスト駆動開発".to_string(),
                description: "テストを先に書いてから実装する".to_string(),
                steps: vec![],
                source_ids: vec!["abc12345".to_string()],
                frequency: 3,
            },
        ],
    }
}

fn make_test_draft() -> SkillDraft {
    SkillDraft {
        name: "rust-dev".to_string(),
        description: "Rust開発。(cargo checkでビルド確認、テスト駆動開発) cargo checkでビルド確認、テスト駆動開発と言われた時に使用。".to_string(),
        body: "# Rust開発\n\nSample body.".to_string(),
        sources: vec!["abc12345".to_string()],
        existing_skill: None,
        diff: None,
    }
}

#[test]
fn format_skill_md_has_yaml_frontmatter() {
    let draft = make_test_draft();
    let md = generator::format_skill_md(&draft);

    assert!(md.starts_with("---\n"), "Should start with YAML frontmatter delimiter");
    assert!(md.contains("name: rust-dev"), "Should contain name field");
    assert!(md.contains("description: \""), "Should contain description field");
    // Frontmatter should be closed
    let parts: Vec<&str> = md.splitn(3, "---").collect();
    assert!(parts.len() >= 3, "Should have opening and closing --- delimiters");
}

#[test]
fn format_skill_md_escapes_quotes_in_description() {
    let mut draft = make_test_draft();
    draft.description = r#"Has "quotes" inside"#.to_string();
    let md = generator::format_skill_md(&draft);
    assert!(
        md.contains(r#"Has \"quotes\" inside"#),
        "Quotes in description should be escaped, got: {}",
        md
    );
}

#[test]
fn format_skill_md_contains_body() {
    let draft = make_test_draft();
    let md = generator::format_skill_md(&draft);
    assert!(
        md.contains("# Rust開発"),
        "Should contain body content"
    );
}

#[test]
fn generate_skills_from_cluster() {
    let cluster = make_test_cluster();
    let drafts = generator::generate_skills(&[cluster]);
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].name, "rust-dev");
}

#[test]
fn generate_skills_empty_patterns() {
    let cluster = DomainCluster {
        domain: "Rust開発".to_string(),
        conversations: vec![],
        patterns: vec![],
    };
    let drafts = generator::generate_skills(&[cluster]);
    assert!(drafts.is_empty(), "Empty patterns should produce no drafts");
}

#[test]
fn check_existing_skills_no_dir() {
    let cluster = make_test_cluster();
    let mut drafts = generator::generate_skills(&[cluster]);
    // Non-existent directory should not error
    let result = generator::check_existing_skills(
        &mut drafts,
        Path::new("/nonexistent/path/to/skills"),
    );
    assert!(result.is_ok());
    assert!(drafts[0].existing_skill.is_none());
}

#[test]
fn check_existing_skills_finds_match() {
    // Create a temp dir with a matching skill file
    let temp_dir = std::env::temp_dir().join("skill-miner-test-skills");
    let _ = std::fs::create_dir_all(&temp_dir);
    let skill_file = temp_dir.join("rust-dev.md");
    std::fs::write(&skill_file, "# Existing skill").unwrap();

    let cluster = make_test_cluster();
    let mut drafts = generator::generate_skills(&[cluster]);
    let result = generator::check_existing_skills(&mut drafts, &temp_dir);
    assert!(result.is_ok());
    assert!(
        drafts[0].existing_skill.is_some(),
        "Should find existing skill by name match"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&temp_dir);
}
