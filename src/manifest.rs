// Manifest module: read/write/update manifest.toml, status transitions
// Issue #21

use crate::error::SkillMinerError;
use crate::types::{DraftEntry, DraftStatus, Manifest};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
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

/// Build lookup maps from clusters for O(1) draft-to-cluster resolution.
/// Returns (domain_slug_map, skill_slug_map).
fn build_cluster_index(
    clusters: &[crate::types::DomainCluster],
) -> (
    HashMap<String, usize>,
    HashMap<String, usize>,
) {
    let mut domain_map = HashMap::new();
    let mut skill_map = HashMap::new();
    for (i, c) in clusters.iter().enumerate() {
        let slug = crate::domains::normalize(&c.domain).slug.clone();
        domain_map.entry(slug).or_insert(i);
        for p in &c.patterns {
            if let Some(ref s) = p.skill_slug {
                skill_map.entry(s.clone()).or_insert(i);
            }
        }
    }
    (domain_map, skill_map)
}

/// Create a manifest from generated skill drafts and domain clusters.
pub fn create_from_drafts(
    drafts: &[crate::types::SkillDraft],
    clusters: &[crate::types::DomainCluster],
    _drafts_dir: &Path,
) -> Manifest {
    use chrono::Utc;

    let (domain_map, skill_map) = build_cluster_index(clusters);

    let mut entries = Vec::new();
    for draft in drafts {
        let cluster = domain_map
            .get(&draft.name)
            .or_else(|| skill_map.get(&draft.name))
            .map(|&i| &clusters[i]);

        // Count patterns that belong to this draft's slug
        let pattern_count = cluster
            .map(|c| {
                let domain_slug = crate::domains::normalize(&c.domain).slug.to_string();
                if domain_slug == draft.name {
                    // Domain-level: all patterns
                    c.patterns.len()
                } else {
                    // Topic-level: only patterns with matching skill_slug
                    c.patterns
                        .iter()
                        .filter(|p| p.skill_slug.as_deref() == Some(&draft.name))
                        .count()
                }
            })
            .unwrap_or(0);

        let conv_count = cluster.map(|c| c.conversations.len()).unwrap_or(0);

        // Compute hash from the file content
        let content = draft.format_md();
        let hash = compute_hash(&content);

        let domain = cluster
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
            score: None,
            fire_count: None,
        });
    }

    Manifest {
        version: "1.0".to_string(),
        generated_at: Utc::now(),
        entries,
        mined_ids: HashSet::new(),
        pending_extracts: Vec::new(),
    }
}

/// Merge new drafts into an existing manifest, preserving existing entries.
/// Updates counts/hash for existing slugs; appends new ones.
pub fn merge_drafts(
    manifest: &mut Manifest,
    drafts: &[crate::types::SkillDraft],
    clusters: &[crate::types::DomainCluster],
) {
    let new_mf = create_from_drafts(drafts, clusters, Path::new(""));

    for new_entry in new_mf.entries {
        if let Some(existing) = manifest.entries.iter_mut().find(|e| e.slug == new_entry.slug) {
            // Update counts/hash, preserve status/deployed_at/score/fire_count
            existing.pattern_count = new_entry.pattern_count;
            existing.conversation_count = new_entry.conversation_count;
            existing.content_hash = new_entry.content_hash;
            existing.generated_at = new_entry.generated_at;
        } else {
            manifest.entries.push(new_entry);
        }
    }

    manifest.generated_at = chrono::Utc::now();
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
            mined_ids: HashSet::new(),
            pending_extracts: Vec::new(),
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
                score: None,
                fire_count: None,
            });
        }
    }

    Ok(Manifest {
        version: "1.0".to_string(),
        generated_at: Utc::now(),
        entries,
        mined_ids: HashSet::new(),
        pending_extracts: Vec::new(),
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
            if desc.is_empty() {
                return None;
            }
            // Domain is typically the first sentence before a period (Japanese or English)
            if let Some(pos) = desc.find('。') {
                return Some(desc[..pos].to_string());
            }
            if let Some(pos) = desc.find('.') {
                return Some(desc[..pos].to_string());
            }
            // No period found: use full description as domain hint
            return Some(desc.to_string());
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
                domain: "Testing & QA".to_string(),
                status: DraftStatus::Draft,
                pattern_count: 3,
                conversation_count: 5,
                generated_at: Utc::now(),
                deployed_at: None,
                content_hash: compute_hash("test content"),
                score: None,
                fire_count: None,
            }],
            mined_ids: HashSet::new(),
            pending_extracts: Vec::new(),
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

    #[test]
    fn manifest_with_mined_ids_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut manifest = make_manifest();
        manifest.mined_ids.insert("conv-abc".to_string());
        manifest.mined_ids.insert("conv-def".to_string());
        write_manifest(dir.path(), &manifest).unwrap();
        let loaded = read_manifest(dir.path()).unwrap();
        assert_eq!(loaded.mined_ids.len(), 2);
        assert!(loaded.mined_ids.contains("conv-abc"));
        assert!(loaded.mined_ids.contains("conv-def"));
    }

    #[test]
    fn manifest_backward_compat_no_mined_ids() {
        // Old manifest without mined_ids should deserialize with empty set
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest();
        assert!(manifest.mined_ids.is_empty());
        write_manifest(dir.path(), &manifest).unwrap();
        // The written TOML should not have mined_ids key (skip_serializing_if)
        let content = std::fs::read_to_string(dir.path().join("manifest.toml")).unwrap();
        assert!(!content.contains("mined_ids"));
        // Reading it back should still work
        let loaded = read_manifest(dir.path()).unwrap();
        assert!(loaded.mined_ids.is_empty());
    }
}
