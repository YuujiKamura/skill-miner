// Progressive mining engine: expands time windows from recent to past,
// stopping when no new (unprocessed) conversations are found.

use crate::types::{
    ClassifiedConversation, Conversation, DomainCluster, Manifest, MineConfig, PipelineStats,
    SkillDraft,
};
use crate::{classifier, compressor, extractor, generator, manifest, parser};
use anyhow::Result;
use chrono::{Duration, Utc};
use std::collections::HashMap;

/// Result of a progressive mining run.
pub struct MineResult {
    pub drafts: Vec<SkillDraft>,
    pub clusters: Vec<DomainCluster>,
    pub stats: PipelineStats,
    pub windows_processed: usize,
    pub new_conversations: usize,
    /// Number of windows that were low-value (below significance threshold)
    pub skipped_low_value: usize,
}

/// Progressive mining configuration.
pub struct ProgressiveConfig {
    /// Maximum days to look back (default: 30)
    pub max_days: u32,
    /// Maximum windows to process (None = unlimited until stop condition)
    pub max_windows: Option<usize>,
    /// Minimum messages per conversation
    pub min_messages: usize,
    /// Maximum parallel AI calls
    pub parallel: usize,
    /// Minimum ratio of significant (non-misc, confidence >= 0.5) conversations
    /// in a window. Below this threshold, mining stops early. (default: 0.3)
    pub min_significance_ratio: f64,
}

/// Run progressive mining: expand time windows from recent to past,
/// stopping when a window yields no new conversations.
pub fn mine_progressive(
    config: &MineConfig,
    manifest: &mut Manifest,
    progressive: &ProgressiveConfig,
    dry_run: bool,
) -> Result<MineResult> {
    let now = Utc::now();
    let max_lookback = Duration::hours(progressive.max_days as i64 * 24);

    // Build time windows: first 12h, then 24h each
    let window_hours = std::iter::once(12i64).chain(std::iter::repeat(24i64));
    let mut cursor_hours: i64 = 0;

    let mut all_classified: Vec<ClassifiedConversation> = Vec::new();
    let mut all_conversations: Vec<Conversation> = Vec::new();
    let mut windows_processed: usize = 0;
    let mut total_classify_calls: usize = 0;
    let mut skipped_low_value: usize = 0;

    for window_size in window_hours {
        let window_start_hours = cursor_hours + window_size;

        // Check max depth
        if Duration::hours(cursor_hours) >= max_lookback {
            eprintln!("Reached max depth ({} days), stopping.", progressive.max_days);
            break;
        }

        // Check max windows
        if let Some(max_w) = progressive.max_windows {
            if windows_processed >= max_w {
                eprintln!("Reached max windows ({}), stopping.", max_w);
                break;
            }
        }

        // Clamp window to max lookback
        let clamped_start_hours = window_start_hours.min(max_lookback.num_hours());

        let end = now - Duration::hours(cursor_hours);
        let start = now - Duration::hours(clamped_start_hours);

        // Parse conversations in this window
        let convs = parser::parse_window(
            &config.projects_dir,
            progressive.min_messages,
            start,
            end,
        )?;

        // Filter out already-mined conversations
        let new_convs: Vec<Conversation> = convs
            .into_iter()
            .filter(|c| !manifest.mined_ids.contains(&c.id))
            .collect();

        if new_convs.is_empty() {
            eprintln!(
                "[window {}] {}h ago → {}h ago: 0 new conversations → stopping",
                windows_processed, clamped_start_hours, cursor_hours
            );
            break;
        }

        eprintln!(
            "[window {}] {}h ago → {}h ago: {} new conversations",
            windows_processed,
            clamped_start_hours,
            cursor_hours,
            new_convs.len()
        );

        // Compress & classify this window's conversations
        let summaries = compressor::compress_all(&new_convs);
        let classified = classifier::classify(&summaries, &config.ai_options)?;

        let classify_calls = (summaries.len() + 49) / 50;
        total_classify_calls += classify_calls;

        // Log domain breakdown
        let groups = classifier::group_by_domain(&classified);
        let mut domain_counts: Vec<_> = groups.iter().map(|(d, cs)| (d.clone(), cs.len())).collect();
        domain_counts.sort_by(|a, b| b.1.cmp(&a.1));
        let breakdown: Vec<String> = domain_counts.iter().map(|(d, n)| format!("{} → {}", d, n)).collect();
        eprintln!("  {}", breakdown.join(", "));

        // Significance check: stop if too many conversations are misc/low-confidence
        let significant_count = classified
            .iter()
            .filter(|c| c.slug != "misc" && c.confidence >= 0.5)
            .count();
        let ratio = if classified.is_empty() {
            0.0
        } else {
            significant_count as f64 / classified.len() as f64
        };

        // Mark conversations as mined
        for c in &new_convs {
            manifest.mined_ids.insert(c.id.clone());
        }

        // Add results before potentially stopping
        all_classified.extend(classified);
        all_conversations.extend(new_convs);
        windows_processed += 1;

        if ratio < progressive.min_significance_ratio {
            skipped_low_value += 1;
            eprintln!(
                "  significance {:.0}% < threshold {:.0}% → stopping",
                ratio * 100.0,
                progressive.min_significance_ratio * 100.0
            );
            break;
        }

        // Advance cursor
        cursor_hours = clamped_start_hours;
    }

    if all_classified.is_empty() {
        eprintln!("No new conversations to process.");
        return Ok(MineResult {
            drafts: Vec::new(),
            clusters: Vec::new(),
            stats: PipelineStats::default(),
            windows_processed,
            new_conversations: 0,
            skipped_low_value,
        });
    }

    let new_conversations = all_conversations.len();

    // Build conv_map for extractor
    let conv_map: HashMap<String, &Conversation> = all_conversations
        .iter()
        .map(|c| (c.id.clone(), c))
        .collect();

    // Group all classified by domain, then extract patterns in parallel
    let groups = classifier::group_by_domain(&all_classified);

    eprintln!(
        "Extracting patterns from {} domains (parallel)...",
        groups.len()
    );
    let (clusters, extract_calls) = extractor::extract_all_parallel(
        &groups,
        Some(&conv_map),
        &config.ai_options,
        progressive.parallel,
    )?;
    for cluster in &clusters {
        eprintln!("  {} → {} patterns", cluster.domain, cluster.patterns.len());
    }

    // Generate skill drafts
    eprintln!("Generating skills...");
    let mut drafts = generator::generate_skills(&clusters);
    if !dry_run {
        generator::check_existing_skills(&mut drafts, &config.skills_dir)?;
    }

    let stats = PipelineStats {
        classify_calls: total_classify_calls,
        extract_calls,
        total_calls: total_classify_calls + extract_calls,
    };

    Ok(MineResult {
        drafts,
        clusters,
        stats,
        windows_processed,
        new_conversations,
        skipped_low_value,
    })
}

/// Merge new drafts into an existing manifest, preserving existing entries.
pub fn merge_into_manifest(
    manifest: &mut Manifest,
    drafts: &[SkillDraft],
    clusters: &[DomainCluster],
) {
    let new_mf = manifest::create_from_drafts(drafts, clusters, std::path::Path::new(""));

    for new_entry in new_mf.entries {
        if let Some(existing) = manifest
            .entries
            .iter_mut()
            .find(|e| e.slug == new_entry.slug)
        {
            // Update counts/hash, preserve status/deployed_at/score/fire_count
            existing.pattern_count = new_entry.pattern_count;
            existing.conversation_count += new_entry.conversation_count;
            existing.content_hash = new_entry.content_hash;
            existing.generated_at = new_entry.generated_at;
            // status, deployed_at, score, fire_count are intentionally NOT overwritten
        } else {
            manifest.entries.push(new_entry);
        }
    }

    manifest.generated_at = chrono::Utc::now();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use std::collections::HashSet;

    fn make_empty_manifest() -> Manifest {
        Manifest {
            version: "1.0".to_string(),
            generated_at: Utc::now(),
            entries: Vec::new(),
            mined_ids: HashSet::new(),
        }
    }

    #[test]
    fn test_merge_into_manifest_new_entry() {
        let mut manifest = make_empty_manifest();
        let drafts = vec![SkillDraft {
            name: "test-skill".to_string(),
            description: "Test skill".to_string(),
            body: "# Test\n\nBody".to_string(),
            sources: vec!["conv1".to_string()],
            existing_skill: None,
            diff: None,
        }];
        let clusters = vec![DomainCluster {
            domain: "test-skill".to_string(),
            conversations: vec![],
            patterns: vec![KnowledgePattern {
                title: "pattern".to_string(),
                description: "desc".to_string(),
                steps: vec![],
                source_ids: vec![],
                frequency: 1,
            }],
        }];

        merge_into_manifest(&mut manifest, &drafts, &clusters);
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].slug, "test-skill");
    }

    #[test]
    fn test_merge_into_manifest_updates_existing() {
        let mut manifest = make_empty_manifest();
        manifest.entries.push(DraftEntry {
            slug: "existing".to_string(),
            domain: "existing".to_string(),
            status: DraftStatus::Approved,
            pattern_count: 2,
            conversation_count: 3,
            generated_at: Utc::now(),
            deployed_at: None,
            content_hash: "old-hash".to_string(),
            score: Some(0.8),
            fire_count: Some(5),
        });

        let drafts = vec![SkillDraft {
            name: "existing".to_string(),
            description: "Updated".to_string(),
            body: "# Updated".to_string(),
            sources: vec![],
            existing_skill: None,
            diff: None,
        }];
        let clusters = vec![DomainCluster {
            domain: "existing".to_string(),
            conversations: vec![],
            patterns: vec![],
        }];

        merge_into_manifest(&mut manifest, &drafts, &clusters);
        // Should still have 1 entry, not 2
        assert_eq!(manifest.entries.len(), 1);
        // Status should be preserved (Approved, not reset to Draft)
        assert_eq!(manifest.entries[0].status, DraftStatus::Approved);
        // Score and fire_count should be preserved
        assert_eq!(manifest.entries[0].score, Some(0.8));
        assert_eq!(manifest.entries[0].fire_count, Some(5));
    }

    #[test]
    fn test_significance_ratio_logic() {
        // Simulate classified conversations and check significance ratio calculation
        let make_classified = |slug: &str, confidence: f64| ClassifiedConversation {
            summary: ConversationSummary {
                id: format!("conv-{}", slug),
                source_path: std::path::PathBuf::from("/tmp/test"),
                first_message: "test".to_string(),
                message_count: 5,
                start_time: None,
                cwd: None,
                topics: vec![],
                tools_used: vec![],
                files_touched: vec![],
                commands_used: vec![],
            },
            domain: slug.to_string(),
            slug: slug.to_string(),
            tags: vec![],
            confidence,
        };

        // All misc → ratio = 0.0
        let classified = vec![
            make_classified("misc", 0.3),
            make_classified("misc", 0.6),
            make_classified("misc", 0.8),
        ];
        let significant = classified
            .iter()
            .filter(|c| c.slug != "misc" && c.confidence >= 0.5)
            .count();
        let ratio = significant as f64 / classified.len() as f64;
        assert_eq!(ratio, 0.0);
        assert!(ratio < 0.3); // Would trigger stop

        // Mixed: 1 significant out of 4 → ratio = 0.25
        let classified = vec![
            make_classified("pavement", 0.9),
            make_classified("misc", 0.3),
            make_classified("misc", 0.6),
            make_classified("misc", 0.8),
        ];
        let significant = classified
            .iter()
            .filter(|c| c.slug != "misc" && c.confidence >= 0.5)
            .count();
        let ratio = significant as f64 / classified.len() as f64;
        assert_eq!(ratio, 0.25);
        assert!(ratio < 0.3); // Would trigger stop

        // Mixed: 2 significant out of 4 → ratio = 0.5
        let classified = vec![
            make_classified("pavement", 0.9),
            make_classified("photo-management", 0.7),
            make_classified("misc", 0.3),
            make_classified("misc", 0.8),
        ];
        let significant = classified
            .iter()
            .filter(|c| c.slug != "misc" && c.confidence >= 0.5)
            .count();
        let ratio = significant as f64 / classified.len() as f64;
        assert_eq!(ratio, 0.5);
        assert!(ratio >= 0.3); // Would NOT trigger stop

        // Non-misc but low confidence → not significant
        let classified = vec![
            make_classified("pavement", 0.3), // low confidence
            make_classified("misc", 0.8),
        ];
        let significant = classified
            .iter()
            .filter(|c| c.slug != "misc" && c.confidence >= 0.5)
            .count();
        let ratio = significant as f64 / classified.len() as f64;
        assert_eq!(ratio, 0.0);
        assert!(ratio < 0.3); // Would trigger stop
    }

    #[test]
    fn test_mined_ids_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let mut manifest = make_empty_manifest();
        manifest.mined_ids.insert("conv-1".to_string());
        manifest.mined_ids.insert("conv-2".to_string());
        manifest.mined_ids.insert("conv-3".to_string());

        crate::manifest::write_manifest(dir.path(), &manifest).unwrap();
        let loaded = crate::manifest::read_manifest(dir.path()).unwrap();
        assert_eq!(loaded.mined_ids.len(), 3);
        assert!(loaded.mined_ids.contains("conv-1"));
        assert!(loaded.mined_ids.contains("conv-2"));
        assert!(loaded.mined_ids.contains("conv-3"));
    }
}
