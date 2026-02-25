// Bundle module: export/import skill packs for portability & trading
// Issue #23

use crate::error::SkillMinerError;
use crate::manifest;
use crate::types::{
    BundleSkill, BundleStats, DraftEntry, DraftStatus, ImportResult, Manifest, SkillBundle,
};
use std::path::Path;

/// Options for exporting a bundle.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Only export approved/deployed skills (skip drafts/rejected)
    pub approved_only: bool,
    /// Bundle name
    pub name: String,
    /// Optional author
    pub author: Option<String>,
    /// Description
    pub description: String,
}

/// Export skills as a portable .skillpack directory.
pub fn export_bundle(
    draft_dir: &Path,
    output: &Path,
    manifest: &Manifest,
    opts: &ExportOptions,
) -> Result<SkillBundle, SkillMinerError> {
    // Filter entries based on options
    let entries: Vec<&DraftEntry> = manifest
        .entries
        .iter()
        .filter(|e| {
            if opts.approved_only {
                matches!(e.status, DraftStatus::Approved | DraftStatus::Deployed)
            } else {
                e.status != DraftStatus::Rejected
            }
        })
        .collect();

    // Create output directory structure
    let skills_dir = output.join("skills");
    std::fs::create_dir_all(&skills_dir)?;

    let mut bundle_skills = Vec::new();
    let mut total_patterns = 0;

    for entry in &entries {
        let source = draft_dir.join(format!("{}.md", entry.slug));
        if !source.exists() {
            eprintln!("warn: draft file missing for {}, skipping", entry.slug);
            continue;
        }

        let content = std::fs::read_to_string(&source)?;
        let hash = manifest::compute_hash(&content);

        // Verify hash matches
        if hash != entry.content_hash {
            eprintln!(
                "warn: hash mismatch for {} (file changed since manifest), using current hash",
                entry.slug
            );
        }

        // Copy to bundle
        let dest = skills_dir.join(format!("{}.md", entry.slug));
        std::fs::write(&dest, &content)?;

        bundle_skills.push(BundleSkill {
            slug: entry.slug.clone(),
            domain: entry.domain.clone(),
            pattern_count: entry.pattern_count,
            content_hash: hash,
        });

        total_patterns += entry.pattern_count;
    }

    let bundle = SkillBundle {
        name: opts.name.clone(),
        version: "1.0".to_string(),
        author: opts.author.clone(),
        description: opts.description.clone(),
        created_at: chrono::Utc::now(),
        source: BundleStats {
            conversations: entries.iter().map(|e| e.conversation_count).sum(),
            domains: entries.len(),
            patterns: total_patterns,
        },
        skills: bundle_skills,
    };

    // Write bundle manifest
    let bundle_toml =
        toml::to_string_pretty(&bundle).map_err(|e| SkillMinerError::Config(e.to_string()))?;
    std::fs::write(output.join("manifest.toml"), bundle_toml)?;

    Ok(bundle)
}

/// Import a .skillpack bundle into the drafts directory.
pub fn import_bundle(
    bundle_path: &Path,
    draft_dir: &Path,
    manifest: &mut Manifest,
) -> Result<ImportResult, SkillMinerError> {
    let bundle = read_bundle(bundle_path)?;

    std::fs::create_dir_all(draft_dir)?;

    let mut result = ImportResult {
        imported: Vec::new(),
        skipped: Vec::new(),
        conflicted: Vec::new(),
    };

    let bundle_skills_dir = bundle_path.join("skills");

    for skill in &bundle.skills {
        let source = bundle_skills_dir.join(format!("{}.md", skill.slug));
        if !source.exists() {
            eprintln!(
                "warn: skill file missing in bundle: {}, skipping",
                skill.slug
            );
            continue;
        }

        let content = std::fs::read_to_string(&source)?;
        let actual_hash = manifest::compute_hash(&content);

        // Verify integrity
        if actual_hash != skill.content_hash {
            eprintln!(
                "warn: hash mismatch for {} in bundle (possibly corrupted)",
                skill.slug
            );
        }

        // Check if already exists in drafts
        let existing = manifest.entries.iter().find(|e| e.slug == skill.slug);

        match existing {
            Some(e) if e.content_hash == actual_hash => {
                // Identical content, skip
                result.skipped.push(skill.slug.clone());
            }
            Some(_) => {
                // Different content, conflict
                // Write with .imported suffix for manual review
                let dest = draft_dir.join(format!("{}.imported.md", skill.slug));
                std::fs::write(&dest, &content)?;
                result.conflicted.push(skill.slug.clone());
            }
            None => {
                // New skill, import
                let dest = draft_dir.join(format!("{}.md", skill.slug));
                std::fs::write(&dest, &content)?;

                manifest.entries.push(DraftEntry {
                    slug: skill.slug.clone(),
                    domain: skill.domain.clone(),
                    status: DraftStatus::Draft,
                    pattern_count: skill.pattern_count,
                    conversation_count: 0,
                    generated_at: chrono::Utc::now(),
                    deployed_at: None,
                    content_hash: actual_hash,
                });

                result.imported.push(skill.slug.clone());
            }
        }
    }

    Ok(result)
}

/// Read a bundle manifest from a .skillpack directory.
pub fn read_bundle(bundle_path: &Path) -> Result<SkillBundle, SkillMinerError> {
    let manifest_path = bundle_path.join("manifest.toml");
    if !manifest_path.exists() {
        return Err(SkillMinerError::Config(format!(
            "bundle manifest not found: {}",
            manifest_path.display()
        )));
    }
    let content = std::fs::read_to_string(&manifest_path)?;
    toml::from_str(&content)
        .map_err(|e| SkillMinerError::Parse(format!("bundle manifest: {}", e)))
}

/// Verify a bundle's integrity by checking all content hashes.
pub fn verify_bundle(bundle_path: &Path) -> Result<Vec<String>, SkillMinerError> {
    let bundle = read_bundle(bundle_path)?;
    let skills_dir = bundle_path.join("skills");
    let mut errors = Vec::new();

    for skill in &bundle.skills {
        let path = skills_dir.join(format!("{}.md", skill.slug));
        if !path.exists() {
            errors.push(format!("{}: file missing", skill.slug));
            continue;
        }

        let content = std::fs::read_to_string(&path)?;
        let actual_hash = manifest::compute_hash(&content);
        if actual_hash != skill.content_hash {
            errors.push(format!(
                "{}: hash mismatch (expected {}, got {})",
                skill.slug,
                &skill.content_hash[..8],
                &actual_hash[..8]
            ));
        }
    }

    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DraftEntry;
    use chrono::Utc;

    fn make_entry(slug: &str, domain: &str, status: DraftStatus) -> DraftEntry {
        let content = format!("---\nname: {}\n---\n\n# {}\n", slug, domain);
        DraftEntry {
            slug: slug.to_string(),
            domain: domain.to_string(),
            status,
            pattern_count: 3,
            conversation_count: 5,
            generated_at: Utc::now(),
            deployed_at: None,
            content_hash: manifest::compute_hash(&content),
        }
    }

    #[test]
    fn export_import_roundtrip() {
        let draft_dir = tempfile::tempdir().unwrap();
        let bundle_dir = tempfile::tempdir().unwrap();
        let import_dir = tempfile::tempdir().unwrap();

        // Create draft files
        let content_a = "---\nname: skill-a\n---\n\n# A\n";
        let content_b = "---\nname: skill-b\n---\n\n# B\n";
        std::fs::write(draft_dir.path().join("skill-a.md"), content_a).unwrap();
        std::fs::write(draft_dir.path().join("skill-b.md"), content_b).unwrap();

        let manifest = Manifest {
            version: "1.0".to_string(),
            generated_at: Utc::now(),
            entries: vec![
                make_entry("skill-a", "A", DraftStatus::Approved),
                make_entry("skill-b", "B", DraftStatus::Approved),
            ],
        };

        // Export
        let opts = ExportOptions {
            approved_only: false,
            name: "test-bundle".to_string(),
            author: Some("tester".to_string()),
            description: "test export".to_string(),
        };
        let bundle = export_bundle(draft_dir.path(), bundle_dir.path(), &manifest, &opts).unwrap();
        assert_eq!(bundle.skills.len(), 2);

        // Import into fresh dir
        let mut import_manifest = Manifest {
            version: "1.0".to_string(),
            generated_at: Utc::now(),
            entries: vec![],
        };

        let result =
            import_bundle(bundle_dir.path(), import_dir.path(), &mut import_manifest).unwrap();
        assert_eq!(result.imported.len(), 2);
        assert!(result.skipped.is_empty());
        assert!(result.conflicted.is_empty());
        assert_eq!(import_manifest.entries.len(), 2);
    }

    #[test]
    fn verify_valid_bundle() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let skills_dir = bundle_dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let content = "test skill content";
        let hash = manifest::compute_hash(content);
        std::fs::write(skills_dir.join("test.md"), content).unwrap();

        let bundle = SkillBundle {
            name: "test".to_string(),
            version: "1.0".to_string(),
            author: None,
            description: "test".to_string(),
            created_at: Utc::now(),
            source: BundleStats {
                conversations: 10,
                domains: 1,
                patterns: 3,
            },
            skills: vec![BundleSkill {
                slug: "test".to_string(),
                domain: "Test".to_string(),
                pattern_count: 3,
                content_hash: hash,
            }],
        };

        let toml_str = toml::to_string_pretty(&bundle).unwrap();
        std::fs::write(bundle_dir.path().join("manifest.toml"), toml_str).unwrap();

        let errors = verify_bundle(bundle_dir.path()).unwrap();
        assert!(errors.is_empty());
    }

    #[test]
    fn verify_corrupt_bundle() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let skills_dir = bundle_dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        std::fs::write(skills_dir.join("test.md"), "modified content").unwrap();

        let bundle = SkillBundle {
            name: "test".to_string(),
            version: "1.0".to_string(),
            author: None,
            description: "test".to_string(),
            created_at: Utc::now(),
            source: BundleStats {
                conversations: 10,
                domains: 1,
                patterns: 3,
            },
            skills: vec![BundleSkill {
                slug: "test".to_string(),
                domain: "Test".to_string(),
                pattern_count: 3,
                content_hash: "wrong_hash".to_string(),
            }],
        };

        let toml_str = toml::to_string_pretty(&bundle).unwrap();
        std::fs::write(bundle_dir.path().join("manifest.toml"), toml_str).unwrap();

        let errors = verify_bundle(bundle_dir.path()).unwrap();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("hash mismatch"));
    }

    #[test]
    fn import_duplicate_detection() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let draft_dir = tempfile::tempdir().unwrap();

        let skills_dir = bundle_dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let content = "same content";
        let hash = manifest::compute_hash(content);
        std::fs::write(skills_dir.join("dup.md"), content).unwrap();

        // Also write to draft dir
        std::fs::write(draft_dir.path().join("dup.md"), content).unwrap();

        let bundle = SkillBundle {
            name: "test".to_string(),
            version: "1.0".to_string(),
            author: None,
            description: "test".to_string(),
            created_at: Utc::now(),
            source: BundleStats {
                conversations: 5,
                domains: 1,
                patterns: 2,
            },
            skills: vec![BundleSkill {
                slug: "dup".to_string(),
                domain: "Test".to_string(),
                pattern_count: 2,
                content_hash: hash.clone(),
            }],
        };

        let toml_str = toml::to_string_pretty(&bundle).unwrap();
        std::fs::write(bundle_dir.path().join("manifest.toml"), toml_str).unwrap();

        let mut manifest = Manifest {
            version: "1.0".to_string(),
            generated_at: Utc::now(),
            entries: vec![DraftEntry {
                slug: "dup".to_string(),
                domain: "Test".to_string(),
                status: DraftStatus::Draft,
                pattern_count: 2,
                conversation_count: 5,
                generated_at: Utc::now(),
                deployed_at: None,
                content_hash: hash,
            }],
        };

        let result =
            import_bundle(bundle_dir.path(), draft_dir.path(), &mut manifest).unwrap();
        assert_eq!(result.skipped.len(), 1);
        assert!(result.imported.is_empty());
    }
}
