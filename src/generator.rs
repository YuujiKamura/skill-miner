use crate::domains;
use crate::error::SkillMinerError;
use crate::types::{DomainCluster, SkillDraft};
use std::collections::HashMap;
use std::path::Path;
use std::fmt;

/// Generate skill drafts from domain clusters
pub fn generate_skills(clusters: &[DomainCluster]) -> Vec<SkillDraft> {
    clusters.iter().flat_map(generate_from_cluster).collect()
}

fn generate_from_cluster(cluster: &DomainCluster) -> Vec<SkillDraft> {
    if cluster.patterns.is_empty() {
        return vec![];
    }

    // Use stable slug from domain master instead of ad-hoc slugify
    let name = domains::normalize(&cluster.domain).slug.to_string();
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

    let domain_def = domains::normalize(&cluster.domain);
    let domain_keywords: Vec<&str> = domain_def.keywords.iter().map(|s| s.as_str()).collect();

    format!(
        "{}。({}) {}と言われた時に使用。",
        cluster.domain,
        pattern_summaries.join("、"),
        domain_keywords.join("、")
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

/// Check existing skills for overlap, read their bodies, and compute diffs
pub fn check_existing_skills(
    drafts: &mut [SkillDraft],
    skills_dir: &Path,
) -> Result<(), SkillMinerError> {
    let existing = load_existing_skills(skills_dir)?;

    // Build a slug→(name, path) lookup for domain master slug matching
    let slug_map: HashMap<String, (&String, &std::path::PathBuf)> = existing
        .iter()
        .map(|(name, path)| {
            let slug = domains::normalize(name).slug.clone();
            (slug, (name, path))
        })
        .collect();

    for draft in drafts.iter_mut() {
        // Check by exact name match
        if let Some(path) = existing.get(&draft.name) {
            draft.existing_skill = Some(path.clone());
        }

        // Check by domain master slug match
        if draft.existing_skill.is_none() {
            let draft_slug = &domains::normalize(&draft.name).slug;
            if let Some((_name, path)) = slug_map.get(draft_slug) {
                draft.existing_skill = Some((*path).clone());
            }
        }

        // Check by substring overlap (original logic)
        if draft.existing_skill.is_none() {
            for (name, path) in &existing {
                if name.contains(&draft.name) || draft.name.contains(name) {
                    draft.existing_skill = Some(path.clone());
                    break;
                }
            }
        }

        // If we found an existing skill, read its body and compute diff
        if let Some(ref path) = draft.existing_skill {
            if let Ok(content) = std::fs::read_to_string(path) {
                let existing_body = extract_body(&content);
                let diff = compute_diff(&existing_body, &draft.body);
                if !diff.lines.is_empty() {
                    draft.diff = Some(diff.to_string());
                }
            }
        }
    }

    Ok(())
}

fn load_existing_skills(skills_dir: &Path) -> Result<HashMap<String, std::path::PathBuf>, SkillMinerError> {
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

/// Extract the body portion of a skill .md file, skipping YAML frontmatter.
/// Frontmatter is delimited by `---` on its own line at the start.
fn extract_body(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() || lines[0].trim() != "---" {
        return content.to_string();
    }
    // Find the closing ---
    if let Some(end) = lines[1..].iter().position(|l| l.trim() == "---") {
        // end is relative to lines[1..], so actual index is end+1
        // Body starts after the closing --- (skip leading blank line)
        let body_start = end + 2; // +1 for offset, +1 for the closing --- line
        let body_lines: Vec<&str> = lines[body_start..]
            .iter()
            .copied()
            .skip_while(|l| l.trim().is_empty())
            .collect();
        body_lines.join("\n")
    } else {
        content.to_string()
    }
}

/// Result of a line-level diff
pub struct DiffResult {
    pub lines: Vec<DiffLine>,
}

pub enum DiffLine {
    Added(String),
    Removed(String),
}

impl DiffResult {
    /// Count of added lines
    pub fn added_count(&self) -> usize {
        self.lines.iter().filter(|l| matches!(l, DiffLine::Added(_))).count()
    }

    /// Count of removed lines
    pub fn removed_count(&self) -> usize {
        self.lines.iter().filter(|l| matches!(l, DiffLine::Removed(_))).count()
    }
}

impl fmt::Display for DiffResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for line in &self.lines {
            match line {
                DiffLine::Added(s) => writeln!(f, "+{}", s)?,
                DiffLine::Removed(s) => writeln!(f, "-{}", s)?,
            }
        }
        Ok(())
    }
}

/// Compute a simple line-level diff between old and new text.
/// Reports lines present in old but not new (removed) and vice versa (added).
/// Uses an ordered set-difference approach on non-empty lines.
fn compute_diff(old: &str, new: &str) -> DiffResult {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut diff_lines = Vec::new();

    // Simple LCS-free approach: walk both line arrays with a two-pointer scan
    // to detect added/removed lines. For our use case (skill body comparison),
    // this gives clear enough results.
    let old_set: HashMap<&str, usize> = {
        let mut m = HashMap::new();
        for line in &old_lines {
            *m.entry(*line).or_insert(0) += 1;
        }
        m
    };
    let new_set: HashMap<&str, usize> = {
        let mut m = HashMap::new();
        for line in &new_lines {
            *m.entry(*line).or_insert(0) += 1;
        }
        m
    };

    // Lines in old but not in new (or fewer occurrences) → removed
    let mut old_counts = old_set.clone();
    for (line, &new_count) in &new_set {
        if let Some(old_count) = old_counts.get_mut(line) {
            if new_count >= *old_count {
                *old_count = 0;
            } else {
                *old_count -= new_count;
            }
        }
    }
    for line in &old_lines {
        if let Some(count) = old_counts.get_mut(line) {
            if *count > 0 {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    diff_lines.push(DiffLine::Removed(line.to_string()));
                    *count -= 1;
                }
            }
        }
    }

    // Lines in new but not in old (or more occurrences) → added
    let mut new_counts = new_set;
    for (line, &old_count) in &old_set {
        if let Some(new_count) = new_counts.get_mut(line) {
            if old_count >= *new_count {
                *new_count = 0;
            } else {
                *new_count -= old_count;
            }
        }
    }
    for line in &new_lines {
        if let Some(count) = new_counts.get_mut(*line) {
            if *count > 0 {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    diff_lines.push(DiffLine::Added(line.to_string()));
                    *count -= 1;
                }
            }
        }
    }

    DiffResult { lines: diff_lines }
}

/// Parse diff summary from a SkillDraft's diff string.
/// Returns (added_count, removed_count).
pub fn parse_diff_summary(diff: &str) -> (usize, usize) {
    let added = diff.lines().filter(|l| l.starts_with('+')).count();
    let removed = diff.lines().filter(|l| l.starts_with('-')).count();
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_body_with_frontmatter() {
        let content = "---\nname: test\ndescription: \"foo\"\n---\n\n# Body\n\nHello world\n";
        let body = extract_body(content);
        assert_eq!(body, "# Body\n\nHello world");
    }

    #[test]
    fn test_extract_body_no_frontmatter() {
        let content = "# No frontmatter\n\nJust body text\n";
        let body = extract_body(content);
        assert_eq!(body, content);
    }

    #[test]
    fn test_compute_diff_identical() {
        let text = "# Title\n\nLine 1\nLine 2\n";
        let diff = compute_diff(text, text);
        assert!(diff.lines.is_empty());
    }

    #[test]
    fn test_compute_diff_additions() {
        let old = "# Title\n\nLine 1\n";
        let new = "# Title\n\nLine 1\nLine 2\n";
        let diff = compute_diff(old, new);
        assert_eq!(diff.added_count(), 1);
        assert_eq!(diff.removed_count(), 0);
        assert!(diff.to_string().contains("+Line 2"));
    }

    #[test]
    fn test_compute_diff_removals() {
        let old = "# Title\n\nLine 1\nLine 2\n";
        let new = "# Title\n\nLine 1\n";
        let diff = compute_diff(old, new);
        assert_eq!(diff.added_count(), 0);
        assert_eq!(diff.removed_count(), 1);
        assert!(diff.to_string().contains("-Line 2"));
    }

    #[test]
    fn test_compute_diff_mixed() {
        let old = "# Title\n\nOld line\nCommon\n";
        let new = "# Title\n\nCommon\nNew line\n";
        let diff = compute_diff(old, new);
        assert_eq!(diff.removed_count(), 1);
        assert_eq!(diff.added_count(), 1);
        assert!(diff.to_string().contains("-Old line"));
        assert!(diff.to_string().contains("+New line"));
    }

    #[test]
    fn test_parse_diff_summary() {
        let diff_str = "+added line 1\n+added line 2\n-removed line\n";
        let (added, removed) = parse_diff_summary(diff_str);
        assert_eq!(added, 2);
        assert_eq!(removed, 1);
    }
}
