// Deployer module: deploy skills, diff, prune
// Issue #22

use crate::domains;
use crate::error::SkillMinerError;
use crate::manifest;
use crate::types::{DeployResult, DraftEntry, DraftStatus, Manifest, PruneOptions, SkillDraft};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

/// Deploy a single skill draft to the skills directory.
pub fn deploy_skill(
    draft_dir: &Path,
    skills_dir: &Path,
    entry: &DraftEntry,
) -> Result<DeployResult, SkillMinerError> {
    let source = draft_dir.join(format!("{}.md", entry.slug));
    if !source.exists() {
        return Err(SkillMinerError::Config(format!(
            "draft file not found: {}",
            source.display()
        )));
    }

    std::fs::create_dir_all(skills_dir)?;

    let target = skills_dir.join(format!("{}.md", entry.slug));
    let was_update = target.exists();

    std::fs::copy(&source, &target)?;

    Ok(DeployResult {
        slug: entry.slug.clone(),
        target_path: target,
        was_update,
    })
}

/// Deploy all approved drafts.
pub fn deploy_approved(
    draft_dir: &Path,
    skills_dir: &Path,
    manifest: &mut Manifest,
) -> Result<Vec<DeployResult>, SkillMinerError> {
    let approved_slugs: Vec<String> = manifest
        .entries
        .iter()
        .filter(|e| e.status == DraftStatus::Approved)
        .map(|e| e.slug.clone())
        .collect();

    let mut results = Vec::new();
    for slug in &approved_slugs {
        let entry = manifest
            .entries
            .iter()
            .find(|e| e.slug == *slug)
            .cloned()
            .unwrap();
        let result = deploy_skill(draft_dir, skills_dir, &entry)?;
        results.push(result);

        // Update manifest
        if let Some(e) = manifest.entries.iter_mut().find(|e| e.slug == *slug) {
            e.status = DraftStatus::Deployed;
            e.deployed_at = Some(chrono::Utc::now());
        }
    }

    Ok(results)
}

/// Deploy specific drafts by slug names.
pub fn deploy_by_names(
    draft_dir: &Path,
    skills_dir: &Path,
    manifest: &mut Manifest,
    names: &[String],
) -> Result<Vec<DeployResult>, SkillMinerError> {
    let mut results = Vec::new();
    for name in names {
        let entry = manifest
            .entries
            .iter()
            .find(|e| e.slug == *name)
            .cloned()
            .ok_or_else(|| SkillMinerError::Config(format!("draft not found: {}", name)))?;
        let result = deploy_skill(draft_dir, skills_dir, &entry)?;
        results.push(result);

        if let Some(e) = manifest.entries.iter_mut().find(|e| e.slug == *name) {
            e.status = DraftStatus::Deployed;
            e.deployed_at = Some(chrono::Utc::now());
        }
    }
    Ok(results)
}

/// Show diff between a draft and its deployed version.
pub fn diff_skill(
    draft_dir: &Path,
    skills_dir: &Path,
    slug: &str,
) -> Result<String, SkillMinerError> {
    let draft_path = draft_dir.join(format!("{}.md", slug));
    let deployed_path = skills_dir.join(format!("{}.md", slug));

    if !draft_path.exists() {
        return Err(SkillMinerError::Config(format!(
            "draft not found: {}",
            slug
        )));
    }

    if !deployed_path.exists() {
        return Ok(format!("[NEW] {} — not yet deployed", slug));
    }

    let draft_content = std::fs::read_to_string(&draft_path)?;
    let deployed_content = std::fs::read_to_string(&deployed_path)?;

    let draft_hash = manifest::compute_hash(&draft_content);
    let deployed_hash = manifest::compute_hash(&deployed_content);

    if draft_hash == deployed_hash {
        return Ok(format!("[IDENTICAL] {} — no changes", slug));
    }

    // Line-level diff
    let draft_lines: Vec<&str> = draft_content.lines().collect();
    let deployed_lines: Vec<&str> = deployed_content.lines().collect();

    let mut diff = String::new();
    diff.push_str(&format!(
        "[CHANGED] {} — draft vs deployed\n",
        slug
    ));

    // Simple unified-style output
    let max_lines = draft_lines.len().max(deployed_lines.len());
    for i in 0..max_lines {
        let d = deployed_lines.get(i).copied().unwrap_or("");
        let n = draft_lines.get(i).copied().unwrap_or("");
        if d != n {
            if !d.is_empty() {
                diff.push_str(&format!("-{}\n", d));
            }
            if !n.is_empty() {
                diff.push_str(&format!("+{}\n", n));
            }
        }
    }

    Ok(diff)
}

/// Prune drafts based on options.
pub fn prune(
    draft_dir: &Path,
    manifest: &mut Manifest,
    opts: &PruneOptions,
) -> Result<Vec<String>, SkillMinerError> {
    let mut removed = Vec::new();

    manifest.entries.retain(|entry| {
        let should_remove = (opts.misc && entry.domain == "その他")
            || (opts.rejected && entry.status == DraftStatus::Rejected)
            || (opts.duplicates && is_duplicate_japanese_name(&entry.slug));

        if should_remove {
            // Try to remove the file
            let path = draft_dir.join(format!("{}.md", entry.slug));
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
            removed.push(entry.slug.clone());
            false // remove from manifest
        } else {
            true // keep
        }
    });

    Ok(removed)
}

// ── Skill overlap checking (moved from generator.rs) ──

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
pub fn extract_body(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() || lines[0].trim() != "---" {
        return content.to_string();
    }
    if let Some(end) = lines[1..].iter().position(|l| l.trim() == "---") {
        let body_start = end + 2;
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
    pub fn added_count(&self) -> usize {
        self.lines.iter().filter(|l| matches!(l, DiffLine::Added(_))).count()
    }

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
fn compute_diff(old: &str, new: &str) -> DiffResult {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut diff_lines = Vec::new();

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

/// Check if a slug looks like a Japanese-named duplicate.
/// Japanese slugs that also have an English equivalent should be pruned.
fn is_duplicate_japanese_name(slug: &str) -> bool {
    // If the slug contains non-ASCII chars, it's a Japanese name
    slug.chars().any(|c| !c.is_ascii())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DraftEntry;
    use chrono::Utc;

    fn make_entry(slug: &str, domain: &str, status: DraftStatus) -> DraftEntry {
        DraftEntry {
            slug: slug.to_string(),
            domain: domain.to_string(),
            status,
            pattern_count: 3,
            conversation_count: 5,
            generated_at: Utc::now(),
            deployed_at: None,
            content_hash: manifest::compute_hash("test"),
            score: None,
            fire_count: None,
        }
    }

    fn make_manifest_with(entries: Vec<DraftEntry>) -> Manifest {
        Manifest {
            version: "1.0".to_string(),
            generated_at: Utc::now(),
            entries,
            mined_ids: std::collections::HashSet::new(),
            pending_extracts: Vec::new(),
        }
    }

    #[test]
    fn deploy_creates_file() {
        let draft_dir = tempfile::tempdir().unwrap();
        let skills_dir = tempfile::tempdir().unwrap();

        // Create a draft file
        let draft_content = "---\nname: test\n---\n\n# Test\n";
        std::fs::write(draft_dir.path().join("test-skill.md"), draft_content).unwrap();

        let entry = make_entry("test-skill", "テスト", DraftStatus::Approved);
        let result = deploy_skill(draft_dir.path(), skills_dir.path(), &entry).unwrap();

        assert!(!result.was_update);
        assert!(result.target_path.exists());
        assert_eq!(
            std::fs::read_to_string(&result.target_path).unwrap(),
            draft_content
        );
    }

    #[test]
    fn deploy_approved_updates_status() {
        let draft_dir = tempfile::tempdir().unwrap();
        let skills_dir = tempfile::tempdir().unwrap();

        std::fs::write(draft_dir.path().join("skill-a.md"), "content a").unwrap();
        std::fs::write(draft_dir.path().join("skill-b.md"), "content b").unwrap();

        let mut manifest = make_manifest_with(vec![
            make_entry("skill-a", "A", DraftStatus::Approved),
            make_entry("skill-b", "B", DraftStatus::Draft),
        ]);

        let results = deploy_approved(draft_dir.path(), skills_dir.path(), &mut manifest).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "skill-a");
        assert_eq!(manifest.entries[0].status, DraftStatus::Deployed);
        assert!(manifest.entries[0].deployed_at.is_some());
        assert_eq!(manifest.entries[1].status, DraftStatus::Draft); // unchanged
    }

    #[test]
    fn prune_removes_misc_and_rejected() {
        let draft_dir = tempfile::tempdir().unwrap();

        std::fs::write(draft_dir.path().join("misc.md"), "misc content").unwrap();
        std::fs::write(draft_dir.path().join("rejected-one.md"), "rejected").unwrap();
        std::fs::write(draft_dir.path().join("good.md"), "good").unwrap();

        let mut manifest = make_manifest_with(vec![
            make_entry("misc", "その他", DraftStatus::Draft),
            make_entry("rejected-one", "テスト", DraftStatus::Rejected),
            make_entry("good", "良い", DraftStatus::Draft),
        ]);

        let opts = PruneOptions {
            misc: true,
            rejected: true,
            duplicates: false,
        };

        let removed = prune(draft_dir.path(), &mut manifest, &opts).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(removed.contains(&"misc".to_string()));
        assert!(removed.contains(&"rejected-one".to_string()));
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].slug, "good");
    }

    #[test]
    fn diff_shows_new() {
        let draft_dir = tempfile::tempdir().unwrap();
        let skills_dir = tempfile::tempdir().unwrap();

        std::fs::write(draft_dir.path().join("new-skill.md"), "new content").unwrap();

        let result = diff_skill(draft_dir.path(), skills_dir.path(), "new-skill").unwrap();
        assert!(result.contains("[NEW]"));
    }

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

    #[test]
    fn diff_shows_identical() {
        let draft_dir = tempfile::tempdir().unwrap();
        let skills_dir = tempfile::tempdir().unwrap();

        let content = "same content";
        std::fs::write(draft_dir.path().join("same.md"), content).unwrap();
        std::fs::write(skills_dir.path().join("same.md"), content).unwrap();

        let result = diff_skill(draft_dir.path(), skills_dir.path(), "same").unwrap();
        assert!(result.contains("[IDENTICAL]"));
    }
}
