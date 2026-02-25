/// Fixed domain master for stable classification.
/// AI picks from this list instead of free-text, ensuring consistent slugs.
///
/// Domains are loaded from `domains.toml` (embedded at compile time).
/// Falls back to a static array if parsing fails.

use serde::Deserialize;
use std::sync::LazyLock;

/// Domain definition (owned strings, deserialized from TOML).
#[derive(Debug, Clone, Deserialize)]
pub struct DomainDef {
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Deserialize)]
struct DomainsFile {
    domain: Vec<DomainDef>,
}

/// Embedded domains.toml content (compile-time).
const DOMAINS_TOML: &str = include_str!("../domains.toml");

/// Parsed domain list, lazily initialized.
/// Falls back to built-in defaults if TOML parsing fails.
static DOMAIN_LIST: LazyLock<Vec<DomainDef>> = LazyLock::new(|| {
    match toml::from_str::<DomainsFile>(DOMAINS_TOML) {
        Ok(file) => file.domain,
        Err(e) => {
            eprintln!("warn: failed to parse domains.toml, using built-in defaults: {e}");
            builtin_domains()
        }
    }
});

/// Access the domain list (static lifetime via LazyLock).
pub fn domains() -> &'static [DomainDef] {
    &DOMAIN_LIST
}

/// Backward-compatible alias: `DOMAINS` still works via this function.
/// Callers that used `DOMAINS.iter()` should migrate to `domains().iter()`.
pub static DOMAINS: &LazyLock<Vec<DomainDef>> = &DOMAIN_LIST;

/// Look up a DomainDef by its name (exact match).
pub fn find_by_name(name: &str) -> Option<&'static DomainDef> {
    domains().iter().find(|d| d.name == name)
}

/// Normalize a free-text domain name to the closest master entry.
/// Tries exact match first, then substring/keyword match, falls back to "その他".
pub fn normalize(raw: &str) -> &'static DomainDef {
    let raw_trimmed = raw.trim();
    let list = domains();

    // Empty string -> misc (avoids false substring match since "" is contained in every string)
    if raw_trimmed.is_empty() {
        return &list[list.len() - 1]; // misc
    }

    // Exact match
    if let Some(d) = find_by_name(raw_trimmed) {
        return d;
    }

    // Substring match: master name is contained in raw, or raw is contained in master name
    // Require at least 2 chars for the "raw contained in master name" direction
    // to avoid false positives with very short strings
    for d in list.iter() {
        if d.name == "その他" {
            continue;
        }
        if raw_trimmed.contains(d.name.as_str())
            || (raw_trimmed.chars().count() >= 2 && d.name.contains(raw_trimmed))
        {
            return d;
        }
    }

    // Keyword match: check if any keyword appears in the raw domain text
    let mut best: Option<(&DomainDef, usize)> = None;
    for d in list.iter() {
        if d.name == "その他" {
            continue;
        }
        let hits = d
            .keywords
            .iter()
            .filter(|kw| raw_trimmed.contains(kw.as_str()))
            .count();
        if hits > 0 {
            if best.map_or(true, |(_, prev)| hits > prev) {
                best = Some((d, hits));
            }
        }
    }
    if let Some((d, _)) = best {
        return d;
    }

    // Fallback
    list.iter()
        .find(|d| d.slug == "misc")
        .unwrap_or(&list[list.len() - 1])
}

/// Build the domain list text to embed in AI prompts.
/// Forces AI to pick from this exact list.
pub fn prompt_domain_list() -> String {
    let mut lines = Vec::new();
    for d in domains().iter() {
        if !d.keywords.is_empty() {
            lines.push(format!("- {}: {}", d.name, d.keywords.join("、")));
        } else {
            lines.push(format!("- {}: 上記に当てはまらないもの", d.name));
        }
    }
    lines.join("\n")
}

/// Built-in fallback domain list (matches the original static DOMAINS).
fn builtin_domains() -> Vec<DomainDef> {
    vec![
        DomainDef {
            name: "舗装工事".into(),
            slug: "pavement".into(),
            keywords: vec![
                "舗装".into(),
                "温度管理".into(),
                "出来形".into(),
                "転圧".into(),
                "切削".into(),
                "アスファルト".into(),
                "路盤".into(),
                "品質管理".into(),
            ],
        },
        DomainDef {
            name: "写真管理".into(),
            slug: "photo-management".into(),
            keywords: vec![
                "写真".into(),
                "工事写真".into(),
                "台帳".into(),
                "タグ".into(),
                "アルバム".into(),
                "撮影".into(),
            ],
        },
        DomainDef {
            name: "PDF操作".into(),
            slug: "pdf".into(),
            keywords: vec![
                "PDF".into(),
                "pdf".into(),
                "生成".into(),
                "結合".into(),
                "テンプレート".into(),
                "書き込み".into(),
            ],
        },
        DomainDef {
            name: "施工体制".into(),
            slug: "construction-admin".into(),
            keywords: vec![
                "施工体制".into(),
                "下請".into(),
                "安全書類".into(),
                "カルテ".into(),
                "台帳".into(),
                "契約".into(),
            ],
        },
        DomainDef {
            name: "スプレッドシート".into(),
            slug: "spreadsheet".into(),
            keywords: vec![
                "スプレッドシート".into(),
                "Excel".into(),
                "Google Sheets".into(),
                "数式".into(),
                "xlsx".into(),
            ],
        },
        DomainDef {
            name: "Rust開発".into(),
            slug: "rust-dev".into(),
            keywords: vec![
                "Rust".into(),
                "クレート".into(),
                "cargo".into(),
                "ビルド".into(),
                "WASM".into(),
                "derive".into(),
            ],
        },
        DomainDef {
            name: "AI連携".into(),
            slug: "ai-integration".into(),
            keywords: vec![
                "Gemini".into(),
                "Claude".into(),
                "API".into(),
                "プロンプト".into(),
                "精度".into(),
                "モデル".into(),
            ],
        },
        DomainDef {
            name: "区画線".into(),
            slug: "lane-marking".into(),
            keywords: vec![
                "区画線".into(),
                "数量計算".into(),
                "レーン".into(),
                "ライン".into(),
            ],
        },
        DomainDef {
            name: "DXF/CAD".into(),
            slug: "dxf-cad".into(),
            keywords: vec!["DXF".into(), "CAD".into(), "横断図".into(), "図面".into()],
        },
        DomainDef {
            name: "工程管理".into(),
            slug: "schedule".into(),
            keywords: vec![
                "工程".into(),
                "週報".into(),
                "スケジュール".into(),
                "工程表".into(),
            ],
        },
        DomainDef {
            name: "ツール設計".into(),
            slug: "tool-design".into(),
            keywords: vec![
                "CLI".into(),
                "スキル".into(),
                "自動化".into(),
                "ツール設計".into(),
            ],
        },
        DomainDef {
            name: "その他".into(),
            slug: "misc".into(),
            keywords: vec![],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_parses_successfully() {
        let file: DomainsFile = toml::from_str(DOMAINS_TOML).expect("domains.toml should parse");
        assert!(!file.domain.is_empty());
        assert!(file.domain.iter().any(|d| d.slug == "pavement"));
        assert!(file.domain.iter().any(|d| d.slug == "misc"));
    }

    #[test]
    fn domains_loaded() {
        let list = domains();
        assert!(list.len() >= 10);
        assert!(list.iter().any(|d| d.slug == "pavement"));
    }

    #[test]
    fn exact_match() {
        let d = normalize("舗装工事");
        assert_eq!(d.slug, "pavement");
    }

    #[test]
    fn substring_match() {
        let d = normalize("舗装工事関連");
        assert_eq!(d.slug, "pavement");
    }

    #[test]
    fn keyword_match() {
        let d = normalize("温度管理と転圧");
        assert_eq!(d.slug, "pavement");
    }

    #[test]
    fn keyword_match_ai() {
        let d = normalize("Geminiプロンプト設計");
        assert_eq!(d.slug, "ai-integration");
    }

    #[test]
    fn fallback_to_misc() {
        let d = normalize("全く関係ない話題");
        assert_eq!(d.slug, "misc");
    }

    #[test]
    fn prompt_list_contains_all_domains() {
        let list = prompt_domain_list();
        for d in domains().iter() {
            assert!(list.contains(d.name.as_str()), "Missing domain: {}", d.name);
        }
    }
}
