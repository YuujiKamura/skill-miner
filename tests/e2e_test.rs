//! End-to-end integration test: JSONL → parse → compress → format_for_classification
//! and fixture-based classify/extract → generate → SkillDraft format verification.
//! No AI calls are made; classification and extraction results are loaded from fixtures.

use skill_miner::classifier;
use skill_miner::compressor;
use skill_miner::generator;
use skill_miner::parser;
use skill_miner::types::{
    ClassifiedConversation, ConversationSummary, DomainCluster, KnowledgePattern,
};
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixture_path() -> PathBuf {
    fixture_dir().join("sample_conversation.jsonl")
}

// --- Pipeline: parse → compress → format_for_classification ---

#[test]
fn e2e_parse_compress_format() {
    // Step 1: Parse conversation from JSONL fixture
    let conv = parser::parse_conversation(&fixture_path()).unwrap();
    assert!(conv.message_count() >= 4, "fixture should have at least 4 messages");

    // Step 2: Compress into summary
    let summaries = compressor::compress_all(&[conv]);
    assert_eq!(summaries.len(), 1);
    let summary = &summaries[0];
    assert!(
        summary.first_message.contains("Fix the build error"),
        "first message should be extracted"
    );
    assert!(summary.message_count >= 4);

    // Step 3: Format for classification
    let formatted = compressor::format_for_classification(&summaries);
    assert!(formatted.contains("[0]"), "should contain index [0]");
    assert!(
        formatted.contains("Fix the build error"),
        "should contain first message text"
    );
    assert!(formatted.contains("msgs="), "should contain message count");
}

// --- Fixture-based classify → group → extract → generate ---

fn make_summary(id: &str, first_msg: &str) -> ConversationSummary {
    ConversationSummary {
        id: id.to_string(),
        source_path: PathBuf::from("/tmp/dummy.jsonl"),
        first_message: first_msg.to_string(),
        message_count: 6,
        start_time: None,
        cwd: Some("/home/user/project".to_string()),
        topics: vec!["rust".to_string(), "file:.rs".to_string()],
        tools_used: vec!["Read".to_string(), "Edit".to_string()],
        files_touched: vec!["src/main.rs".to_string()],
        commands_used: vec!["cargo check".to_string()],
    }
}

#[test]
fn e2e_fixture_classify_to_generate() {
    // Load classify fixture (simulates AI classification response)
    let classify_json = std::fs::read_to_string(fixture_dir().join("classify_response.json")).unwrap();

    #[derive(serde::Deserialize)]
    struct ClassifyEntry {
        index: usize,
        domain: String,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        confidence: f64,
    }

    let entries: Vec<ClassifyEntry> = serde_json::from_str(&classify_json).unwrap();

    // Build summaries for each classified entry
    let summaries: Vec<ConversationSummary> = vec![
        make_summary("conv001", "cargo checkでビルドエラーを修正して"),
        make_summary("conv002", "Geminiプロンプトを最適化して"),
        make_summary("conv003", "PDFの結合処理を実装して"),
    ];

    // Simulate classification results by combining entries with summaries
    let classified: Vec<ClassifiedConversation> = entries
        .iter()
        .filter(|e| e.index < summaries.len())
        .map(|e| {
            let domain_def = skill_miner::domains::normalize(&e.domain);
            ClassifiedConversation {
                summary: summaries[e.index].clone(),
                domain: domain_def.name.to_string(),
                slug: domain_def.slug.to_string(),
                tags: e.tags.clone(),
                confidence: e.confidence,
            }
        })
        .collect();

    assert_eq!(classified.len(), 3);

    // Group by domain
    let groups = classifier::group_by_domain(&classified);
    assert!(!groups.is_empty(), "should have at least one domain group");

    // Load extract fixture (simulates AI pattern extraction)
    let extract_json = std::fs::read_to_string(fixture_dir().join("extract_response.json")).unwrap();

    #[derive(serde::Deserialize)]
    struct PatternEntry {
        title: String,
        description: String,
        #[serde(default)]
        steps: Vec<String>,
        #[serde(default = "default_freq")]
        frequency: usize,
    }
    fn default_freq() -> usize { 1 }

    let patterns: Vec<PatternEntry> = serde_json::from_str(&extract_json).unwrap();
    assert_eq!(patterns.len(), 2);

    // Build DomainCluster from fixture data
    let rust_convs: Vec<ClassifiedConversation> = classified
        .iter()
        .filter(|c| c.domain == "Rust開発")
        .cloned()
        .collect();

    let cluster = DomainCluster {
        domain: "Rust開発".to_string(),
        conversations: rust_convs,
        patterns: patterns
            .into_iter()
            .map(|p| KnowledgePattern {
                title: p.title,
                description: p.description,
                steps: p.steps,
                source_ids: vec!["conv001".to_string()],
                frequency: p.frequency,
            })
            .collect(),
    };

    // Generate skill drafts
    let drafts = generator::generate_skills(&[cluster]);
    assert_eq!(drafts.len(), 1, "should generate exactly one skill draft");

    let draft = &drafts[0];
    assert_eq!(draft.name, "rust-dev", "slug should be rust-dev");
    assert!(
        !draft.description.is_empty(),
        "description should not be empty"
    );
    assert!(
        !draft.body.is_empty(),
        "body should not be empty"
    );
    assert!(
        draft.body.contains("# Rust開発"),
        "body should contain domain heading"
    );
    assert!(
        draft.body.contains("cargo check"),
        "body should contain pattern content"
    );

    // Format as markdown and verify structure
    let md = generator::format_skill_md(draft);
    assert!(md.starts_with("---\n"), "should start with YAML frontmatter");
    assert!(md.contains("name: rust-dev"), "should contain name field");
    assert!(
        md.contains("description: \""),
        "should contain quoted description"
    );
    // Verify frontmatter is closed
    let parts: Vec<&str> = md.splitn(3, "---").collect();
    assert!(
        parts.len() >= 3,
        "should have opening and closing --- delimiters"
    );
}

#[test]
fn e2e_empty_patterns_no_drafts() {
    let cluster = DomainCluster {
        domain: "AI連携".to_string(),
        conversations: vec![],
        patterns: vec![],
    };
    let drafts = generator::generate_skills(&[cluster]);
    assert!(
        drafts.is_empty(),
        "empty patterns should produce no skill drafts"
    );
}

#[test]
fn e2e_multiple_clusters_generate_multiple_drafts() {
    let cluster1 = DomainCluster {
        domain: "Rust開発".to_string(),
        conversations: vec![],
        patterns: vec![KnowledgePattern {
            title: "パターンA".to_string(),
            description: "説明A".to_string(),
            steps: vec![],
            source_ids: vec!["id1".to_string()],
            frequency: 2,
        }],
    };
    let cluster2 = DomainCluster {
        domain: "PDF操作".to_string(),
        conversations: vec![],
        patterns: vec![KnowledgePattern {
            title: "パターンB".to_string(),
            description: "説明B".to_string(),
            steps: vec!["手順1".to_string()],
            source_ids: vec!["id2".to_string()],
            frequency: 1,
        }],
    };

    let drafts = generator::generate_skills(&[cluster1, cluster2]);
    assert_eq!(drafts.len(), 2);

    let slugs: Vec<&str> = drafts.iter().map(|d| d.name.as_str()).collect();
    assert!(slugs.contains(&"rust-dev"), "should have rust-dev slug");
    assert!(slugs.contains(&"pdf"), "should have pdf slug");
}

#[test]
fn e2e_skill_draft_format_contains_steps() {
    let cluster = DomainCluster {
        domain: "Rust開発".to_string(),
        conversations: vec![],
        patterns: vec![KnowledgePattern {
            title: "ビルド確認".to_string(),
            description: "cargo checkで確認".to_string(),
            steps: vec![
                "コードを変更".to_string(),
                "cargo checkを実行".to_string(),
                "エラーを修正".to_string(),
            ],
            source_ids: vec!["s1".to_string()],
            frequency: 5,
        }],
    };

    let drafts = generator::generate_skills(&[cluster]);
    let body = &drafts[0].body;

    assert!(body.contains("### 手順"), "body should contain steps header");
    assert!(body.contains("1. コードを変更"), "step 1 should be numbered");
    assert!(
        body.contains("2. cargo checkを実行"),
        "step 2 should be numbered"
    );
    assert!(body.contains("出現頻度: 5回"), "frequency should be shown");
}
