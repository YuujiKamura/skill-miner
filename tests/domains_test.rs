use skill_miner::domains;

#[test]
fn normalize_empty_string() {
    // Empty string should fall back to misc, not match every domain via substring
    let d = domains::normalize("");
    assert_eq!(d.slug, "misc");
}

#[test]
fn normalize_whitespace_only() {
    // After trim, becomes empty string -> falls back to misc
    let d = domains::normalize("   ");
    assert_eq!(d.slug, "misc");
}

#[test]
fn normalize_unknown_domain_falls_back_to_misc() {
    let d = domains::normalize("space exploration");
    assert_eq!(d.slug, "misc");
}

#[test]
fn normalize_exact_match_web_dev() {
    let d = domains::normalize("Web Development");
    assert_eq!(d.slug, "web-dev");
    assert_eq!(d.name, "Web Development");
}

#[test]
fn normalize_exact_match_ai_ml() {
    let d = domains::normalize("AI & Machine Learning");
    assert_eq!(d.slug, "ai-ml");
}

#[test]
fn normalize_substring_match() {
    // "Web Development projects" contains "Web Development"
    let d = domains::normalize("Web Development projects");
    assert_eq!(d.slug, "web-dev");
}

#[test]
fn normalize_keyword_match_single() {
    // "Docker" is a keyword for DevOps & Infrastructure
    let d = domains::normalize("Docker container setup");
    assert_eq!(d.slug, "devops");
}

#[test]
fn normalize_keyword_match_multiple_picks_best() {
    // "React" and "CSS" both map to Web Development
    let d = domains::normalize("React CSS styling guide");
    assert_eq!(d.slug, "web-dev");
}

#[test]
fn normalize_keyword_match_ai() {
    let d = domains::normalize("LLM prompt engineering");
    assert_eq!(d.slug, "ai-ml");
}

#[test]
fn normalize_keyword_match_database() {
    let d = domains::normalize("PostgreSQL query optimization");
    assert_eq!(d.slug, "database");
}

#[test]
fn normalize_keyword_match_testing() {
    let d = domains::normalize("unit test coverage report");
    assert_eq!(d.slug, "testing");
}

#[test]
fn normalize_misc_slug_is_stable() {
    let d = domains::normalize("completely unrelated string xyz");
    assert_eq!(d.slug, "misc");
    assert_eq!(d.name, "Miscellaneous");
}

#[test]
fn find_by_name_returns_none_for_unknown() {
    assert!(domains::find_by_name("Nonexistent Domain").is_none());
}

#[test]
fn find_by_name_returns_some_for_exact() {
    let d = domains::find_by_name("Web Development").unwrap();
    assert_eq!(d.slug, "web-dev");
}

#[test]
fn prompt_domain_list_contains_all() {
    let list = domains::prompt_domain_list();
    for d in domains::domains().iter() {
        assert!(
            list.contains(d.name.as_str()),
            "prompt_domain_list should contain: {}",
            d.name
        );
    }
}
