/// Fixed domain master for stable classification.
/// AI picks from this list instead of free-text, ensuring consistent slugs.

pub struct DomainDef {
    pub name: &'static str,
    pub slug: &'static str,
    pub keywords: &'static [&'static str],
}

pub static DOMAINS: &[DomainDef] = &[
    DomainDef {
        name: "舗装工事",
        slug: "pavement",
        keywords: &["舗装", "温度管理", "出来形", "転圧", "切削", "アスファルト", "路盤", "品質管理"],
    },
    DomainDef {
        name: "写真管理",
        slug: "photo-management",
        keywords: &["写真", "工事写真", "台帳", "タグ", "アルバム", "撮影"],
    },
    DomainDef {
        name: "PDF操作",
        slug: "pdf",
        keywords: &["PDF", "pdf", "生成", "結合", "テンプレート", "書き込み"],
    },
    DomainDef {
        name: "施工体制",
        slug: "construction-admin",
        keywords: &["施工体制", "下請", "安全書類", "カルテ", "台帳", "契約"],
    },
    DomainDef {
        name: "スプレッドシート",
        slug: "spreadsheet",
        keywords: &["スプレッドシート", "Excel", "Google Sheets", "数式", "xlsx"],
    },
    DomainDef {
        name: "Rust開発",
        slug: "rust-dev",
        keywords: &["Rust", "クレート", "cargo", "ビルド", "WASM", "derive"],
    },
    DomainDef {
        name: "AI連携",
        slug: "ai-integration",
        keywords: &["Gemini", "Claude", "API", "プロンプト", "精度", "モデル"],
    },
    DomainDef {
        name: "区画線",
        slug: "lane-marking",
        keywords: &["区画線", "数量計算", "レーン", "ライン"],
    },
    DomainDef {
        name: "DXF/CAD",
        slug: "dxf-cad",
        keywords: &["DXF", "CAD", "横断図", "図面"],
    },
    DomainDef {
        name: "工程管理",
        slug: "schedule",
        keywords: &["工程", "週報", "スケジュール", "工程表"],
    },
    DomainDef {
        name: "ツール設計",
        slug: "tool-design",
        keywords: &["CLI", "スキル", "自動化", "ツール設計"],
    },
    DomainDef {
        name: "その他",
        slug: "misc",
        keywords: &[],
    },
];

/// Look up a DomainDef by its name (exact match).
pub fn find_by_name(name: &str) -> Option<&'static DomainDef> {
    DOMAINS.iter().find(|d| d.name == name)
}

/// Normalize a free-text domain name to the closest master entry.
/// Tries exact match first, then substring/keyword match, falls back to "その他".
pub fn normalize(raw: &str) -> &'static DomainDef {
    let raw_trimmed = raw.trim();

    // Exact match
    if let Some(d) = find_by_name(raw_trimmed) {
        return d;
    }

    // Substring match: master name is contained in raw, or raw is contained in master name
    for d in DOMAINS.iter() {
        if d.name == "その他" {
            continue;
        }
        if raw_trimmed.contains(d.name) || d.name.contains(raw_trimmed) {
            return d;
        }
    }

    // Keyword match: check if any keyword appears in the raw domain text
    let mut best: Option<(&DomainDef, usize)> = None;
    for d in DOMAINS.iter() {
        if d.name == "その他" {
            continue;
        }
        let hits = d.keywords.iter().filter(|kw| raw_trimmed.contains(**kw)).count();
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
    DOMAINS.iter().find(|d| d.slug == "misc").unwrap()
}

/// Build the domain list text to embed in AI prompts.
/// Forces AI to pick from this exact list.
pub fn prompt_domain_list() -> String {
    let mut lines = Vec::new();
    for d in DOMAINS.iter() {
        if !d.keywords.is_empty() {
            lines.push(format!("- {}: {}", d.name, d.keywords.join("、")));
        } else {
            lines.push(format!("- {}: 上記に当てはまらないもの", d.name));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for d in DOMAINS.iter() {
            assert!(list.contains(d.name), "Missing domain: {}", d.name);
        }
    }
}
