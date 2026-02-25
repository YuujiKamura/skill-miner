use crate::types::{DomainCluster, SkillDraft};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

/// Generate skill drafts from domain clusters
pub fn generate_skills(clusters: &[DomainCluster]) -> Vec<SkillDraft> {
    clusters.iter().flat_map(generate_from_cluster).collect()
}

fn generate_from_cluster(cluster: &DomainCluster) -> Vec<SkillDraft> {
    if cluster.patterns.is_empty() {
        return vec![];
    }

    // Group patterns by similarity (for now, one skill per cluster)
    let name = slugify(&cluster.domain);
    let description = build_description(cluster);
    let body = build_body(cluster);
    let sources: Vec<String> = cluster
        .patterns
        .iter()
        .flat_map(|p| p.source_ids.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    vec![SkillDraft {
        name,
        description,
        body,
        sources,
        existing_skill: None,
        diff: None,
    }]
}

fn build_description(cluster: &DomainCluster) -> String {
    let pattern_summaries: Vec<&str> = cluster
        .patterns
        .iter()
        .take(5)
        .map(|p| p.title.as_str())
        .collect();

    let keywords: Vec<&str> = cluster
        .patterns
        .iter()
        .take(5)
        .map(|p| p.title.as_str())
        .collect();

    format!(
        "{}。({}) {}と言われた時に使用。",
        cluster.domain,
        pattern_summaries.join("、"),
        keywords.join("、")
    )
}

fn build_body(cluster: &DomainCluster) -> String {
    let mut body = format!("# {}\n\n", cluster.domain);

    body.push_str(&format!(
        "会話数: {} | パターン数: {}\n\n",
        cluster.conversations.len(),
        cluster.patterns.len()
    ));

    for (i, pattern) in cluster.patterns.iter().enumerate() {
        body.push_str(&format!("## {}. {}\n\n", i + 1, pattern.title));
        body.push_str(&format!("{}\n\n", pattern.description));

        if !pattern.steps.is_empty() {
            body.push_str("### 手順\n\n");
            for (j, step) in pattern.steps.iter().enumerate() {
                body.push_str(&format!("{}. {}\n", j + 1, step));
            }
            body.push_str("\n");
        }

        body.push_str(&format!("出現頻度: {}回\n\n", pattern.frequency));
    }

    body
}

/// Format a skill draft as a complete .md file
pub fn format_skill_md(draft: &SkillDraft) -> String {
    format!(
        r#"---
name: {}
description: "{}"
---

{}
"#,
        draft.name,
        draft.description.replace('"', r#"\""#),
        draft.body
    )
}

/// Check existing skills for overlap
pub fn check_existing_skills(
    drafts: &mut [SkillDraft],
    skills_dir: &Path,
) -> Result<()> {
    let existing = load_existing_skills(skills_dir)?;

    for draft in drafts.iter_mut() {
        // Check by name match
        if let Some(path) = existing.get(&draft.name) {
            draft.existing_skill = Some(path.clone());
        }

        // Check by domain keyword overlap
        for (name, path) in &existing {
            if name.contains(&draft.name) || draft.name.contains(name) {
                if draft.existing_skill.is_none() {
                    draft.existing_skill = Some(path.clone());
                }
            }
        }
    }

    Ok(())
}

fn load_existing_skills(skills_dir: &Path) -> Result<HashMap<String, std::path::PathBuf>> {
    let mut skills = HashMap::new();

    if !skills_dir.exists() {
        return Ok(skills);
    }

    for entry in std::fs::read_dir(skills_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() && path.extension().map(|e| e == "md").unwrap_or(false) {
            let name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            skills.insert(name, path);
        } else if path.is_dir() {
            let skill_md = path.join("SKILL.md");
            if skill_md.exists() {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                skills.insert(name, skill_md);
            }
        }
    }

    Ok(skills)
}

fn slugify(s: &str) -> String {
    // Simple slugification: lowercase, replace spaces/special chars with hyphens
    let mut slug = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if c == ' ' || c == '_' || c == '/' {
            if !slug.ends_with('-') {
                slug.push('-');
            }
        }
        // Skip non-ASCII (Japanese) - use as-is for now
        else if !c.is_ascii() {
            slug.push(c);
        }
    }
    slug.trim_matches('-').to_string()
}
