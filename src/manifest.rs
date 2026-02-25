// Manifest module: read/write/update manifest.toml, status transitions
// Issue #21

use crate::error::SkillMinerError;
use crate::types::{DraftEntry, DraftStatus, Manifest};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Compute SHA256 hash of content, returned as hex string.
pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

/// Read manifest.toml from a drafts directory.
pub fn read_manifest(dir: &Path) -> Result<Manifest, SkillMinerError> {
    let path = dir.join("manifest.toml");
    let content = std::fs::read_to_string(&path)?;
    toml::from_str(&content).map_err(|e| SkillMinerError::Parse(format!("manifest.toml: {}", e)))
}

/// Write manifest.toml to a drafts directory.
pub fn write_manifest(dir: &Path, manifest: &Manifest) -> Result<(), SkillMinerError> {
    let path = dir.join("manifest.toml");
    let content =
        toml::to_string_pretty(manifest).map_err(|e| SkillMinerError::Config(e.to_string()))?;
    std::fs::write(&path, content)?;
    Ok(())
}

/// Find an entry in the manifest by slug.
pub fn find_entry<'a>(manifest: &'a Manifest, slug: &str) -> Option<&'a DraftEntry> {
    manifest.entries.iter().find(|e| e.slug == slug)
}

/// Find a mutable entry in the manifest by slug.
pub fn find_entry_mut<'a>(manifest: &'a mut Manifest, slug: &str) -> Option<&'a mut DraftEntry> {
    manifest.entries.iter_mut().find(|e| e.slug == slug)
}

/// Update the status of a draft entry. Enforces valid transitions.
pub fn update_status(
    manifest: &mut Manifest,
    slug: &str,
    new_status: DraftStatus,
) -> Result<(), SkillMinerError> {
    let entry = find_entry_mut(manifest, slug)
        .ok_or_else(|| SkillMinerError::Config(format!("draft not found: {}", slug)))?;

    validate_transition(&entry.status, &new_status)?;
    entry.status = new_status;
    Ok(())
}

/// Validate a status transition.
fn validate_transition(
    from: &DraftStatus,
    to: &DraftStatus,
) -> Result<(), SkillMinerError> {
    let valid = matches!(
        (from, to),
        (DraftStatus::Draft, DraftStatus::Approved)
            | (DraftStatus::Draft, DraftStatus::Rejected)
            | (DraftStatus::Approved, DraftStatus::Deployed)
            | (DraftStatus::Approved, DraftStatus::Draft) // un-approve
            | (DraftStatus::Rejected, DraftStatus::Draft) // reconsider
            | (DraftStatus::Deployed, DraftStatus::Draft) // re-generate
    );
    if valid {
        Ok(())
    } else {
        Err(SkillMinerError::Config(format!(
            "invalid transition: {} → {}",
            from, to
        )))
    }
}

/// Create a manifest from generated skill drafts and domain clusters.
pub fn create_from_drafts(
    drafts: &[crate::types::SkillDraft],
    clusters: &[crate::types::DomainCluster],
    _drafts_dir: &Path,
) -> Manifest {
    use chrono::Utc;

    let mut entries = Vec::new();
    for draft in drafts {
        // Find matching cluster for conversation count
        let conv_count = clusters
            .iter()
            .find(|c| {
                crate::domains::normalize(&c.domain).slug == draft.name
            })
            .map(|c| c.conversations.len())
            .unwrap_or(0);

        let pattern_count = clusters
            .iter()
            .find(|c| {
                crate::domains::normalize(&c.domain).slug == draft.name
            })
            .map(|c| c.patterns.len())
            .unwrap_or(0);

        // Compute hash from the file content
        let content = crate::generator::format_skill_md(draft);
        let hash = compute_hash(&content);

        // Find domain name from clusters
        let domain = clusters
            .iter()
            .find(|c| {
                crate::domains::normalize(&c.domain).slug == draft.name
            })
            .map(|c| c.domain.clone())
            .unwrap_or_else(|| draft.name.clone());

        entries.push(DraftEntry {
            slug: draft.name.clone(),
            domain,
            status: DraftStatus::Draft,
            pattern_count,
            conversation_count: conv_count,
            generated_at: Utc::now(),
            deployed_at: None,
            content_hash: hash,
        });
    }

    Manifest {
        version: "1.0".to_string(),
        generated_at: Utc::now(),
        entries,
    }
}

/// Scan .md files in a directory and create a manifest (fallback for legacy dirs without manifest.toml).
pub fn create_from_directory(dir: &Path) -> Result<Manifest, SkillMinerError> {
    use chrono::Utc;

    let mut entries = Vec::new();

    if !dir.exists() {
        return Ok(Manifest {
            version: "1.0".to_string(),
            generated_at: Utc::now(),
            entries,
        });
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "md").unwrap_or(false) {
            let slug = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let content = std::fs::read_to_string(&path)?;
            let hash = compute_hash(&content);

            // Try to extract domain from frontmatter
            let domain = extract_domain_from_frontmatter(&content).unwrap_or_else(|| slug.clone());

            // Count pattern sections (## N.)
            let pattern_count = content
                .lines()
                .filter(|l| {
                    l.starts_with("## ") && l.chars().nth(3).map(|c| c.is_ascii_digit()).unwrap_or(false)
                })
                .count();

            entries.push(DraftEntry {
                slug,
                domain,
                status: DraftStatus::Draft,
                pattern_count,
                conversation_count: 0,
                generated_at: Utc::now(),
                deployed_at: None,
                content_hash: hash,
            });
        }
    }

    Ok(Manifest {
        version: "1.0".to_string(),
        generated_at: Utc::now(),
        entries,
    })
}

/// Extract domain name from YAML frontmatter description field.
fn extract_domain_from_frontmatter(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.first()?.trim() != "---" {
        return None;
    }
    for line in &lines[1..] {
        if line.trim() == "---" {
            break;
        }
        if let Some(rest) = line.strip_prefix("description:") {
            let desc = rest.trim().trim_matches('"');
            // Domain is typically the first word before the period
            if let Some(dot_pos) = desc.find('。') {
                return Some(desc[..dot_pos].to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_manifest() -> Manifest {
        Manifest {
            version: "1.0".to_string(),
            generated_at: Utc::now(),
            entries: vec![DraftEntry {
                slug: "test-skill".to_string(),
                domain: "テスト".to_string(),
                status: DraftStatus::Draft,
                pattern_count: 3,
                conversation_count: 5,
                generated_at: Utc::now(),
                deployed_at: None,
                content_hash: compute_hash("test content"),
            }],
        }
    }

    #[test]
    fn hash_deterministic() {
        let h1 = compute_hash("hello world");
        let h2 = compute_hash("hello world");
        assert_eq!(h1, h2);
        assert_ne!(h1, compute_hash("different"));
    }

    #[test]
    fn manifest_roundtrip_toml() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest();
        write_manifest(dir.path(), &manifest).unwrap();
        let loaded = read_manifest(dir.path()).unwrap();
        assert_eq!(loaded.version, "1.0");
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].slug, "test-skill");
    }

    #[test]
    fn status_transition_valid() {
        let mut manifest = make_manifest();
        update_status(&mut manifest, "test-skill", DraftStatus::Approved).unwrap();
        assert_eq!(manifest.entries[0].status, DraftStatus::Approved);
        update_status(&mut manifest, "test-skill", DraftStatus::Deployed).unwrap();
        assert_eq!(manifest.entries[0].status, DraftStatus::Deployed);
    }

    #[test]
    fn status_transition_invalid() {
        let mut manifest = make_manifest();
        update_status(&mut manifest, "test-skill", DraftStatus::Approved).unwrap();
        let result = update_status(&mut manifest, "test-skill", DraftStatus::Rejected);
        assert!(result.is_err());
    }

    #[test]
    fn find_entry_works() {
        let manifest = make_manifest();
        assert!(find_entry(&manifest, "test-skill").is_some());
        assert!(find_entry(&manifest, "nonexistent").is_none());
    }
}
