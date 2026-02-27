// Bundle module: export/import skill packs for portability & trading
// Issue #23

use crate::error::SkillMinerError;
use crate::graph;
use crate::manifest;
use crate::types::{
    BundleSkill, BundleStats, DraftEntry, DraftStatus, ImportResult, Manifest, SkillBundle,
};
use std::path::{Path, PathBuf};

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
    /// Include referenced memory/context files in bundle
    pub include_context: bool,
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

        // Extract dependency references from skill content
        let deps: Vec<String> = graph::extract_refs(&content)
            .into_iter()
            .map(|r| r.target.clone())
            .collect();

        bundle_skills.push(BundleSkill {
            slug: entry.slug.clone(),
            domain: entry.domain.clone(),
            pattern_count: entry.pattern_count,
            content_hash: hash,
            score: entry.score,
            fire_count: entry.fire_count,
            deployed_at: entry.deployed_at,
            dependencies: deps,
        });

        total_patterns += entry.pattern_count;
    }

    // Copy context files if requested
    if opts.include_context {
        copy_context_files(&bundle_skills, output)?;
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
        context_imported: Vec::new(),
        context_conflicted: Vec::new(),
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
                    score: None,
                    fire_count: None,
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

/// Copy referenced memory/context files into the bundle's context/ directory.
/// Only includes memory files (1 hop: direct refs + their direct refs).
fn copy_context_files(
    skills: &[BundleSkill],
    output: &Path,
) -> Result<(), SkillMinerError> {
    let home = crate::util::home_dir();

    // Collect all memory file references from all skills
    let mut memory_paths: Vec<PathBuf> = Vec::new();
    for skill in skills {
        for dep in &skill.dependencies {
            // Only include memory files (paths containing "memory/")
            if dep.contains("memory/") {
                // Try to resolve relative to home/.claude/projects/*/memory/
                let candidate = resolve_memory_path(&home, dep);
                if let Some(p) = candidate {
                    if p.exists() && !memory_paths.contains(&p) {
                        memory_paths.push(p);
                    }
                }
            }
        }
    }

    if memory_paths.is_empty() {
        return Ok(());
    }

    // 1-hop transitive: also include files referenced by the collected memory files
    let mut transitive: Vec<PathBuf> = Vec::new();
    for mp in &memory_paths {
        if let Ok(content) = std::fs::read_to_string(mp) {
            for raw in graph::extract_refs(&content) {
                if raw.ref_type == crate::types::DepType::MarkdownLink {
                    let parent = mp.parent().unwrap_or(Path::new("."));
                    let resolved = parent.join(&raw.target);
                    if resolved.exists() && !memory_paths.contains(&resolved) && !transitive.contains(&resolved) {
                        transitive.push(resolved);
                    }
                }
            }
        }
    }
    memory_paths.extend(transitive);

    // Copy to context/memory/
    let ctx_dir = output.join("context").join("memory");
    std::fs::create_dir_all(&ctx_dir)?;

    for mp in &memory_paths {
        if let Some(fname) = mp.file_name() {
            let dest = ctx_dir.join(fname);
            std::fs::copy(mp, &dest)?;
        }
    }

    Ok(())
}

/// Try to resolve a memory file reference to an absolute path.
fn resolve_memory_path(home: &Path, dep: &str) -> Option<PathBuf> {
    // Direct path: memory/foo.md -> search in all project memory dirs
    let dep_clean = dep.replace('\\', "/");

    // Try as relative path from home/.claude/projects/*/memory/
    let projects_dir = home.join(".claude").join("projects");
    if projects_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&projects_dir) {
            for entry in entries.flatten() {
                let mem_dir = entry.path().join("memory");
                if mem_dir.is_dir() {
                    // Try the full dep path
                    let candidate = mem_dir.join(dep_clean.trim_start_matches("memory/"));
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
        }
    }

    // Try as absolute path
    let as_path = PathBuf::from(&dep_clean);
    if as_path.exists() {
        return Some(as_path);
    }

    None
}

/// Import context files from a bundle into the current project's memory directory.
pub fn import_context(
    bundle_path: &Path,
    memory_dir: &Path,
    result: &mut ImportResult,
) -> Result<(), SkillMinerError> {
    let ctx_dir = bundle_path.join("context").join("memory");
    if !ctx_dir.exists() {
        return Ok(());
    }

    std::fs::create_dir_all(memory_dir)?;

    let entries = std::fs::read_dir(&ctx_dir)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let fname = match path.file_name() {
            Some(f) => f.to_string_lossy().to_string(),
            None => continue,
        };

        let dest = memory_dir.join(&fname);
        if dest.exists() {
            // Check if content is identical
            let existing = std::fs::read_to_string(&dest).unwrap_or_default();
            let incoming = std::fs::read_to_string(&path)?;
            if existing == incoming {
                // Identical, skip
                continue;
            }
            // Conflict: save with .imported suffix before extension
            let conflict_name = if fname.ends_with(".md") {
                fname.replace(".md", ".imported.md")
            } else {
                format!("{}.imported", fname)
            };
            let conflict_dest = memory_dir.join(&conflict_name);
            std::fs::copy(&path, &conflict_dest)?;
            result.context_conflicted.push(fname);
        } else {
            std::fs::copy(&path, &dest)?;
            result.context_imported.push(fname);
        }
    }

    Ok(())
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
            score: None,
            fire_count: None,
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
            mined_ids: std::collections::HashSet::new(),
            pending_extracts: Vec::new(),
        };

        // Export
        let opts = ExportOptions {
            approved_only: false,
            name: "test-bundle".to_string(),
            author: Some("tester".to_string()),
            description: "test export".to_string(),
            include_context: false,
        };
        let bundle = export_bundle(draft_dir.path(), bundle_dir.path(), &manifest, &opts).unwrap();
        assert_eq!(bundle.skills.len(), 2);

        // Import into fresh dir
        let mut import_manifest = Manifest {
            version: "1.0".to_string(),
            generated_at: Utc::now(),
            entries: vec![],
            mined_ids: std::collections::HashSet::new(),
            pending_extracts: Vec::new(),
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
                score: None,
                fire_count: None,
                deployed_at: None,
                dependencies: vec![],
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
                score: None,
                fire_count: None,
                deployed_at: None,
                dependencies: vec![],
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
                score: None,
                fire_count: None,
                deployed_at: None,
                dependencies: vec![],
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
                score: None,
                fire_count: None,
            }],
            mined_ids: std::collections::HashSet::new(),
            pending_extracts: Vec::new(),
        };

        let result =
            import_bundle(bundle_dir.path(), draft_dir.path(), &mut manifest).unwrap();
        assert_eq!(result.skipped.len(), 1);
        assert!(result.imported.is_empty());
    }

    #[test]
    fn export_preserves_metadata() {
        let draft_dir = tempfile::tempdir().unwrap();
        let bundle_dir = tempfile::tempdir().unwrap();

        let content = "---\nname: scored\n---\n\n# Scored Skill\n";
        std::fs::write(draft_dir.path().join("scored.md"), content).unwrap();

        let manifest = Manifest {
            version: "1.0".to_string(),
            generated_at: Utc::now(),
            entries: vec![DraftEntry {
                slug: "scored".to_string(),
                domain: "test".to_string(),
                status: DraftStatus::Deployed,
                pattern_count: 5,
                conversation_count: 10,
                generated_at: Utc::now(),
                deployed_at: Some(Utc::now()),
                content_hash: manifest::compute_hash(content),
                score: Some(0.85),
                fire_count: Some(12),
            }],
            mined_ids: std::collections::HashSet::new(),
            pending_extracts: Vec::new(),
        };

        let opts = ExportOptions {
            approved_only: false,
            name: "meta-test".to_string(),
            author: None,
            description: "test metadata".to_string(),
            include_context: false,
        };

        let bundle = export_bundle(draft_dir.path(), bundle_dir.path(), &manifest, &opts).unwrap();
        assert_eq!(bundle.skills.len(), 1);

        let skill = &bundle.skills[0];
        assert_eq!(skill.score, Some(0.85));
        assert_eq!(skill.fire_count, Some(12));
        assert!(skill.deployed_at.is_some());
    }

    #[test]
    fn import_context_files() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let memory_dir = tempfile::tempdir().unwrap();

        // Create context/memory/ in bundle
        let ctx_dir = bundle_dir.path().join("context").join("memory");
        std::fs::create_dir_all(&ctx_dir).unwrap();
        std::fs::write(ctx_dir.join("patterns.md"), "# Patterns\nSome content").unwrap();
        std::fs::write(ctx_dir.join("notes.md"), "# Notes\nOther content").unwrap();

        let mut result = ImportResult {
            imported: vec![],
            skipped: vec![],
            conflicted: vec![],
            context_imported: vec![],
            context_conflicted: vec![],
        };

        import_context(bundle_dir.path(), memory_dir.path(), &mut result).unwrap();

        assert_eq!(result.context_imported.len(), 2);
        assert!(memory_dir.path().join("patterns.md").exists());
        assert!(memory_dir.path().join("notes.md").exists());
    }

    #[test]
    fn import_context_conflict() {
        let bundle_dir = tempfile::tempdir().unwrap();
        let memory_dir = tempfile::tempdir().unwrap();

        // Create context/memory/ in bundle
        let ctx_dir = bundle_dir.path().join("context").join("memory");
        std::fs::create_dir_all(&ctx_dir).unwrap();
        std::fs::write(ctx_dir.join("existing.md"), "new content").unwrap();

        // Create existing file in memory dir
        std::fs::write(memory_dir.path().join("existing.md"), "old content").unwrap();

        let mut result = ImportResult {
            imported: vec![],
            skipped: vec![],
            conflicted: vec![],
            context_imported: vec![],
            context_conflicted: vec![],
        };

        import_context(bundle_dir.path(), memory_dir.path(), &mut result).unwrap();

        assert_eq!(result.context_conflicted.len(), 1);
        assert!(memory_dir.path().join("existing.imported.md").exists());
        // Original unchanged
        let original = std::fs::read_to_string(memory_dir.path().join("existing.md")).unwrap();
        assert_eq!(original, "old content");
    }

    #[test]
    fn backward_compat_old_bundle() {
        // Old bundles without score/fire_count/deployed_at/dependencies should still parse
        let bundle_dir = tempfile::tempdir().unwrap();
        let skills_dir = bundle_dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let content = "old skill content";
        let hash = manifest::compute_hash(content);
        std::fs::write(skills_dir.join("old.md"), content).unwrap();

        // Write TOML without new fields (simulating old format)
        let toml_content = format!(
            r#"name = "old-bundle"
version = "1.0"
description = "old format"
created_at = "2025-01-01T00:00:00Z"

[source]
conversations = 5
domains = 1
patterns = 2

[[skills]]
slug = "old"
domain = "Test"
pattern_count = 2
content_hash = "{}"
"#,
            hash
        );
        std::fs::write(bundle_dir.path().join("manifest.toml"), toml_content).unwrap();

        // Should parse without error
        let bundle = read_bundle(bundle_dir.path()).unwrap();
        assert_eq!(bundle.skills.len(), 1);
        assert_eq!(bundle.skills[0].score, None);
        assert_eq!(bundle.skills[0].fire_count, None);
        assert_eq!(bundle.skills[0].deployed_at, None);
        assert!(bundle.skills[0].dependencies.is_empty());

        // Verify should also work
        let errors = verify_bundle(bundle_dir.path()).unwrap();
        assert!(errors.is_empty());
    }
}
