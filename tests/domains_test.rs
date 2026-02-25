use skill_miner::domains;

#[test]
fn normalize_empty_string() {
    // Empty string is a substring of every domain name, so it matches the first
    // non-misc domain in the master list (舗装工事/pavement).
    // This documents the current behavior.
    let d = domains::normalize("");
    assert_eq!(d.slug, "pavement");
}

#[test]
fn normalize_whitespace_only() {
    // After trim, becomes empty string -> same behavior as empty string.
    let d = domains::normalize("   ");
    assert_eq!(d.slug, "pavement");
}

#[test]
fn normalize_unknown_domain_falls_back_to_misc() {
    let d = domains::normalize("宇宙開発");
    assert_eq!(d.slug, "misc");
}

#[test]
fn normalize_exact_match_pavement() {
    let d = domains::normalize("舗装工事");
    assert_eq!(d.slug, "pavement");
    assert_eq!(d.name, "舗装工事");
}

#[test]
fn normalize_exact_match_photo() {
    let d = domains::normalize("写真管理");
    assert_eq!(d.slug, "photo-management");
}

#[test]
fn normalize_substring_match() {
    // "舗装工事関連" contains "舗装工事"
    let d = domains::normalize("舗装工事関連");
    assert_eq!(d.slug, "pavement");
}

#[test]
fn normalize_keyword_match_single() {
    // "転圧" is a keyword for 舗装工事
    let d = domains::normalize("転圧作業の手順");
    assert_eq!(d.slug, "pavement");
}

#[test]
fn normalize_keyword_match_multiple_picks_best() {
    // "温度管理" and "転圧" both map to 舗装工事
    let d = domains::normalize("温度管理と転圧の関係");
    assert_eq!(d.slug, "pavement");
}

#[test]
fn normalize_keyword_match_ai() {
    let d = domains::normalize("Geminiのプロンプト設計");
    assert_eq!(d.slug, "ai-integration");
}

#[test]
fn normalize_keyword_match_pdf() {
    let d = domains::normalize("PDF生成ツール");
    assert_eq!(d.slug, "pdf");
}

#[test]
fn normalize_keyword_match_spreadsheet() {
    let d = domains::normalize("Excelマクロの修正");
    assert_eq!(d.slug, "spreadsheet");
}

#[test]
fn normalize_misc_slug_is_stable() {
    let d = domains::normalize("完全に無関係な文字列xyz");
    assert_eq!(d.slug, "misc");
    assert_eq!(d.name, "その他");
}

#[test]
fn find_by_name_returns_none_for_unknown() {
    assert!(domains::find_by_name("存在しない分野").is_none());
}

#[test]
fn find_by_name_returns_some_for_exact() {
    let d = domains::find_by_name("舗装工事").unwrap();
    assert_eq!(d.slug, "pavement");
}

#[test]
fn prompt_domain_list_contains_all() {
    let list = domains::prompt_domain_list();
    for d in domains::DOMAINS.iter() {
        assert!(
            list.contains(d.name),
            "prompt_domain_list should contain: {}",
            d.name
        );
    }
}
