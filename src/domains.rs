/// Fixed domain master for stable classification.
/// AI picks from this list instead of free-text, ensuring consistent slugs.
///
/// Domains are loaded in this priority:
/// 1. Runtime config: `~/.config/skill-miner/domains.toml`
/// 2. Embedded `domains.toml` (compile-time)
/// 3. Built-in static array (fallback)

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
/// Tries runtime config first, then embedded TOML, then built-in defaults.
static DOMAIN_LIST: LazyLock<Vec<DomainDef>> = LazyLock::new(|| {
    // 1. Try runtime config file
    if let Some(config_path) = runtime_config_path() {
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            match toml::from_str::<DomainsFile>(&content) {
                Ok(file) if !file.domain.is_empty() => return file.domain,
                Ok(_) => eprintln!("warn: runtime domains.toml is empty, falling back"),
                Err(e) => eprintln!("warn: failed to parse runtime domains.toml: {e}"),
            }
        }
    }

    // 2. Embedded compile-time TOML
    match toml::from_str::<DomainsFile>(DOMAINS_TOML) {
        Ok(file) => file.domain,
        Err(e) => {
            eprintln!("warn: failed to parse domains.toml, using built-in defaults: {e}");
            builtin_domains()
        }
    }
});

/// Return the path to the runtime config file, if the directory exists.
fn runtime_config_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("skill-miner").join("domains.toml"))
}

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
/// Tries exact match first, then substring/keyword match, falls back to "Miscellaneous".
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
        if d.slug == "misc" {
            continue;
        }
        if raw_trimmed.contains(d.name.as_str())
            || (raw_trimmed.chars().count() >= 2 && d.name.contains(raw_trimmed))
        {
            return d;
        }
    }

    // Keyword match: check if any keyword appears in the raw domain text (case-insensitive)
    let raw_lower = raw_trimmed.to_lowercase();
    let mut best: Option<(&DomainDef, usize)> = None;
    for d in list.iter() {
        if d.slug == "misc" {
            continue;
        }
        let hits = d
            .keywords
            .iter()
            .filter(|kw| raw_lower.contains(&kw.to_lowercase()))
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
            lines.push(format!("- {}: {}", d.name, d.keywords.join(", ")));
        } else {
            lines.push(format!("- {}: Anything not matching the above", d.name));
        }
    }
    lines.join("\n")
}

/// Built-in fallback domain list.
fn builtin_domains() -> Vec<DomainDef> {
    vec![
        DomainDef {
            name: "Web Development".into(),
            slug: "web-dev".into(),
            keywords: vec![
                "React".into(),
                "Vue".into(),
                "Next.js".into(),
                "CSS".into(),
                "HTML".into(),
                "frontend".into(),
                "backend".into(),
                "API".into(),
                "REST".into(),
                "GraphQL".into(),
            ],
        },
        DomainDef {
            name: "DevOps & Infrastructure".into(),
            slug: "devops".into(),
            keywords: vec![
                "Docker".into(),
                "Kubernetes".into(),
                "CI/CD".into(),
                "deploy".into(),
                "AWS".into(),
                "GCP".into(),
                "Azure".into(),
                "terraform".into(),
                "nginx".into(),
            ],
        },
        DomainDef {
            name: "Database & Storage".into(),
            slug: "database".into(),
            keywords: vec![
                "SQL".into(),
                "PostgreSQL".into(),
                "MySQL".into(),
                "Redis".into(),
                "MongoDB".into(),
                "migration".into(),
                "query".into(),
                "schema".into(),
            ],
        },
        DomainDef {
            name: "AI & Machine Learning".into(),
            slug: "ai-ml".into(),
            keywords: vec![
                "LLM".into(),
                "GPT".into(),
                "Claude".into(),
                "Gemini".into(),
                "prompt".into(),
                "model".into(),
                "training".into(),
                "inference".into(),
                "embedding".into(),
            ],
        },
        DomainDef {
            name: "Testing & QA".into(),
            slug: "testing".into(),
            keywords: vec![
                "test".into(),
                "unit test".into(),
                "integration".into(),
                "coverage".into(),
                "mock".into(),
                "assertion".into(),
                "TDD".into(),
                "fixture".into(),
            ],
        },
        DomainDef {
            name: "CLI & Tooling".into(),
            slug: "cli-tooling".into(),
            keywords: vec![
                "CLI".into(),
                "script".into(),
                "automation".into(),
                "tool".into(),
                "plugin".into(),
                "extension".into(),
                "config".into(),
            ],
        },
        DomainDef {
            name: "Documentation".into(),
            slug: "docs".into(),
            keywords: vec![
                "README".into(),
                "docs".into(),
                "documentation".into(),
                "tutorial".into(),
                "guide".into(),
                "comment".into(),
            ],
        },
        DomainDef {
            name: "Miscellaneous".into(),
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
        assert!(file.domain.iter().any(|d| d.slug == "web-dev"));
        assert!(file.domain.iter().any(|d| d.slug == "misc"));
    }

    #[test]
    fn domains_loaded() {
        let list = domains();
        assert!(list.len() >= 6);
        assert!(list.iter().any(|d| d.slug == "web-dev"));
    }

    #[test]
    fn exact_match() {
        let d = normalize("Web Development");
        assert_eq!(d.slug, "web-dev");
    }

    #[test]
    fn substring_match() {
        let d = normalize("Web Development projects");
        assert_eq!(d.slug, "web-dev");
    }

    #[test]
    fn keyword_match() {
        let d = normalize("React and Vue frontend");
        assert_eq!(d.slug, "web-dev");
    }

    #[test]
    fn keyword_match_ai() {
        let d = normalize("LLM prompt engineering");
        assert_eq!(d.slug, "ai-ml");
    }

    #[test]
    fn fallback_to_misc() {
        let d = normalize("something completely unrelated xyz");
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
