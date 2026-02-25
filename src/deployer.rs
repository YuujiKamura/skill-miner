// Deployer module: deploy skills, diff, prune
// Issue #22

use crate::error::SkillMinerError;
use crate::manifest;
use crate::types::{DeployResult, DraftEntry, DraftStatus, Manifest, PruneOptions};
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
        }
    }

    fn make_manifest_with(entries: Vec<DraftEntry>) -> Manifest {
        Manifest {
            version: "1.0".to_string(),
            generated_at: Utc::now(),
            entries,
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
