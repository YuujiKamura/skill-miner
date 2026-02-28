// Progressive mining engine: expands time windows from recent to past,
// stopping when no new (unprocessed) conversations are found.

use crate::types::{
    ClassifiedConversation, Conversation, DomainCluster, Manifest, MineConfig, PipelineStats,
    SkillDraft,
};
use crate::{classifier, compressor, extractor, generator, manifest, parser};
use anyhow::Result;
use chrono::{Duration, Utc};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Number of consecutive empty windows before stopping progressive mining.
pub const DEFAULT_MAX_CONSECUTIVE_EMPTY: usize = 2;

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
    /// Number of consecutive empty windows before stopping (default: 2)
    pub max_consecutive_empty: usize,
}

/// Result of classifying a single window's conversations.
struct WindowClassification {
    classified: Vec<ClassifiedConversation>,
    classify_calls: usize,
    /// Ratio of significant (non-misc, confidence >= 0.5) conversations
    significance_ratio: f64,
}

/// Result of the extraction phase.
struct ExtractionResult {
    clusters: Vec<DomainCluster>,
    extract_calls: usize,
    extract_failures: usize,
    /// Conversations whose domains failed extraction (to retry later)
    still_pending: Vec<ClassifiedConversation>,
}

/// Classify a window's conversations: compress, classify, log domain breakdown,
/// and compute significance ratio.
fn classify_window(
    conversations: &[Conversation],
    config: &MineConfig,
) -> Result<WindowClassification> {
    let summaries = compressor::compress_all(conversations);
    let classified = classifier::classify(&summaries, &config.ai_options)?;

    let classify_calls = (summaries.len() + 49) / 50;

    // Log domain breakdown
    let groups = classifier::group_by_domain(&classified);
    let mut domain_counts: Vec<_> = groups.iter().map(|(d, cs)| (d.clone(), cs.len())).collect();
    domain_counts.sort_by(|a, b| b.1.cmp(&a.1));
    let breakdown: Vec<String> = domain_counts
        .iter()
        .map(|(d, n)| format!("{} → {}", d, n))
        .collect();
    eprintln!("  {}", breakdown.join(", "));

    // Significance check
    let significant_count = classified
        .iter()
        .filter(|c| c.slug != "misc" && c.confidence >= 0.5)
        .count();
    let significance_ratio = if classified.is_empty() {
        0.0
    } else {
        significant_count as f64 / classified.len() as f64
    };

    Ok(WindowClassification {
        classified,
        classify_calls,
        significance_ratio,
    })
}

/// Run extraction on classified conversations, using parallel domain-level calls.
/// Returns clusters, call counts, and conversations whose domains failed.
fn run_extraction(
    all_for_extract: Vec<ClassifiedConversation>,
    all_conversations: &[Conversation],
    config: &MineConfig,
    parallel: usize,
) -> Result<ExtractionResult> {
    // Build conv_map only from newly parsed conversations
    // (pending retries won't be here — extractor falls back to source_path parsing)
    let conv_map: HashMap<String, &Conversation> = all_conversations
        .iter()
        .map(|c| (c.id.clone(), c))
        .collect();

    let groups = classifier::group_by_domain(&all_for_extract);

    eprintln!(
        "Extracting patterns from {} domains (parallel)...",
        groups.len()
    );
    let (clusters, extract_calls, failed_domains) = extractor::extract_all_parallel(
        &groups,
        Some(&conv_map),
        &config.ai_options,
        parallel,
        config.summarize_options.as_ref(),
    )?;
    let extract_failures = failed_domains.len();
    if extract_failures > 0 {
        eprintln!(
            "  {} domain(s) failed (timeout etc), {} succeeded",
            extract_failures,
            clusters.len()
        );
    }

    // Rebuild pending: keep only failed domains' conversations
    let failed_set: HashSet<&str> = failed_domains.iter().map(|s| s.as_str()).collect();
    let still_pending: Vec<ClassifiedConversation> = all_for_extract
        .into_iter()
        .filter(|c| failed_set.contains(c.slug.as_str()))
        .collect();

    for cluster in &clusters {
        eprintln!("  {} → {} patterns", cluster.domain, cluster.patterns.len());
    }

    Ok(ExtractionResult {
        clusters,
        extract_calls,
        extract_failures,
        still_pending,
    })
}

/// Update manifest after extraction: mark succeeded conversations as mined,
/// store failed conversations as pending for retry.
fn update_manifest_after_extraction(
    manifest: &mut Manifest,
    clusters: &[DomainCluster],
    still_pending: Vec<ClassifiedConversation>,
) {
    // Mark succeeded conversations as mined
    for cluster in clusters {
        for conv in &cluster.conversations {
            manifest.mined_ids.insert(conv.summary.id.clone());
        }
    }

    manifest.pending_extracts = still_pending;

    if !manifest.pending_extracts.is_empty() {
        eprintln!(
            "  {} conversations pending retry (failed domains)",
            manifest.pending_extracts.len()
        );
    }
}

/// Accumulated results from scanning time windows.
struct WindowScanResult {
    classified: Vec<ClassifiedConversation>,
    conversations: Vec<Conversation>,
    windows_processed: usize,
    total_classify_calls: usize,
    skipped_low_value: usize,
}

/// Whether a window loop iteration should continue or stop.
enum WindowAction {
    Continue,
    Stop,
}

/// Parse conversations in a time window and filter out already-processed ones.
fn load_window_conversations(
    config: &MineConfig,
    min_messages: usize,
    cursor_hours: i64,
    clamped_start_hours: i64,
    mined_ids: &HashSet<String>,
    pending_ids: &HashSet<String>,
) -> Result<Vec<Conversation>> {
    let now = Utc::now();
    let end = now - Duration::hours(cursor_hours);
    let start = now - Duration::hours(clamped_start_hours);

    let convs = parser::parse_window(&config.projects_dir, min_messages, start, end)?;

    Ok(convs
        .into_iter()
        .filter(|c| !mined_ids.contains(&c.id) && !pending_ids.contains(&c.id))
        .collect())
}

/// Log window status and handle empty-window streak logic.
/// Returns `WindowAction::Stop` if the consecutive-empty threshold is reached.
fn handle_empty_window(
    windows_processed: usize,
    clamped_start_hours: i64,
    cursor_hours: i64,
    consecutive_empty: &mut usize,
    max_consecutive_empty: usize,
) -> WindowAction {
    *consecutive_empty += 1;
    eprintln!(
        "[window {}] {}h ago \u{2192} {}h ago: 0 new conversations (empty streak: {})",
        windows_processed, clamped_start_hours, cursor_hours, *consecutive_empty
    );
    if *consecutive_empty >= max_consecutive_empty {
        eprintln!(
            "  {} consecutive empty windows \u{2192} stopping",
            max_consecutive_empty
        );
        WindowAction::Stop
    } else {
        WindowAction::Continue
    }
}

/// Log a non-empty window and classify its conversations.
/// Returns `WindowAction::Stop` if significance is below threshold.
fn classify_and_log_window(
    new_convs: &[Conversation],
    config: &MineConfig,
    windows_processed: usize,
    clamped_start_hours: i64,
    cursor_hours: i64,
    min_significance_ratio: f64,
    scan: &mut WindowScanResult,
) -> Result<WindowAction> {
    eprintln!(
        "[window {}] {}h ago \u{2192} {}h ago: {} new conversations",
        windows_processed, clamped_start_hours, cursor_hours, new_convs.len()
    );

    let window_result = classify_window(new_convs, config)?;
    scan.total_classify_calls += window_result.classify_calls;
    scan.classified.extend(window_result.classified);

    if window_result.significance_ratio < min_significance_ratio {
        scan.skipped_low_value += 1;
        eprintln!(
            "  significance {:.0}% < threshold {:.0}% \u{2192} stopping",
            window_result.significance_ratio * 100.0,
            min_significance_ratio * 100.0
        );
        Ok(WindowAction::Stop)
    } else {
        Ok(WindowAction::Continue)
    }
}

/// Scan time windows from recent to past, parsing and classifying conversations.
/// Stops when max depth/windows reached or consecutive empty windows exceed threshold.
fn scan_windows(
    config: &MineConfig,
    manifest: &Manifest,
    progressive: &ProgressiveConfig,
) -> Result<WindowScanResult> {
    let max_lookback = Duration::hours(progressive.max_days as i64 * 24);

    let pending_ids: HashSet<String> = manifest
        .pending_extracts
        .iter()
        .map(|c| c.summary.id.clone())
        .collect();

    let window_hours = std::iter::once(12i64).chain(std::iter::repeat(24i64));
    let mut cursor_hours: i64 = 0;
    let mut consecutive_empty: usize = 0;
    let max_consecutive_empty = progressive.max_consecutive_empty;

    let mut scan = WindowScanResult {
        classified: Vec::new(),
        conversations: Vec::new(),
        windows_processed: 0,
        total_classify_calls: 0,
        skipped_low_value: 0,
    };

    for window_size in window_hours {
        let window_start_hours = cursor_hours + window_size;

        // Check max depth
        if Duration::hours(cursor_hours) >= max_lookback {
            eprintln!("Reached max depth ({} days), stopping.", progressive.max_days);
            break;
        }

        // Check max windows
        if let Some(max_w) = progressive.max_windows {
            if scan.windows_processed >= max_w {
                eprintln!("Reached max windows ({}), stopping.", max_w);
                break;
            }
        }

        let clamped_start_hours = window_start_hours.min(max_lookback.num_hours());

        let new_convs = load_window_conversations(
            config,
            progressive.min_messages,
            cursor_hours,
            clamped_start_hours,
            &manifest.mined_ids,
            &pending_ids,
        )?;

        if new_convs.is_empty() {
            let action = handle_empty_window(
                scan.windows_processed,
                clamped_start_hours,
                cursor_hours,
                &mut consecutive_empty,
                max_consecutive_empty,
            );
            scan.windows_processed += 1;
            cursor_hours = clamped_start_hours;
            if matches!(action, WindowAction::Stop) {
                break;
            }
            continue;
        }

        consecutive_empty = 0;

        // Classify and check significance; collect conversations before potential stop
        let action = classify_and_log_window(
            &new_convs,
            config,
            scan.windows_processed,
            clamped_start_hours,
            cursor_hours,
            progressive.min_significance_ratio,
            &mut scan,
        )?;

        scan.conversations.extend(new_convs);
        scan.windows_processed += 1;

        if matches!(action, WindowAction::Stop) {
            break;
        }

        cursor_hours = clamped_start_hours;
    }

    Ok(scan)
}

/// Run extraction, update manifest, generate skill drafts, and build final stats.
fn finalize_mining(
    config: &MineConfig,
    manifest: &mut Manifest,
    manifest_dir: &Path,
    scan: WindowScanResult,
    parallel: usize,
    dry_run: bool,
) -> Result<MineResult> {
    // Merge pending + newly classified
    let mut all_for_extract: Vec<ClassifiedConversation> =
        manifest.pending_extracts.drain(..).collect();
    all_for_extract.extend(scan.classified);

    if all_for_extract.is_empty() {
        eprintln!("No new conversations to process.");
        return Ok(MineResult {
            drafts: Vec::new(),
            clusters: Vec::new(),
            stats: PipelineStats::default(),
            windows_processed: scan.windows_processed,
            new_conversations: 0,
            skipped_low_value: scan.skipped_low_value,
        });
    }

    let new_conversations = scan.conversations.len();

    // Save checkpoint: store all_for_extract in manifest before extraction
    manifest.pending_extracts = all_for_extract.clone();
    manifest::write_manifest(manifest_dir, manifest)?;

    // Extract patterns from classified conversations
    let extraction = run_extraction(all_for_extract, &scan.conversations, config, parallel)?;

    // Update manifest with extraction results
    update_manifest_after_extraction(manifest, &extraction.clusters, extraction.still_pending);

    // Generate skill drafts
    eprintln!("Generating skills...");
    let mut drafts = generator::generate_skills(&extraction.clusters);
    if !dry_run {
        generator::check_existing_skills(&mut drafts, &config.skills_dir)?;
    }

    let stats = PipelineStats {
        classify_calls: scan.total_classify_calls,
        extract_calls: extraction.extract_calls,
        extract_failures: extraction.extract_failures,
        total_calls: scan.total_classify_calls + extraction.extract_calls,
    };

    Ok(MineResult {
        drafts,
        clusters: extraction.clusters,
        stats,
        windows_processed: scan.windows_processed,
        new_conversations,
        skipped_low_value: scan.skipped_low_value,
    })
}

/// Run progressive mining: expand time windows from recent to past,
/// stopping when a window yields no new conversations.
/// Supports pending_extracts: conversations classified but not yet extracted
/// (e.g. due to timeout) are retried on the next run.
pub fn mine_progressive(
    config: &MineConfig,
    manifest: &mut Manifest,
    progressive: &ProgressiveConfig,
    dry_run: bool,
    manifest_dir: &Path,
) -> Result<MineResult> {
    let scan = scan_windows(config, manifest, progressive)?;
    finalize_mining(config, manifest, manifest_dir, scan, progressive.parallel, dry_run)
}

/// Merge new drafts into an existing manifest, preserving existing entries.
/// Delegates to `manifest::merge_drafts()`.
pub fn merge_into_manifest(
    manifest: &mut Manifest,
    drafts: &[SkillDraft],
    clusters: &[DomainCluster],
) {
    crate::manifest::merge_drafts(manifest, drafts, clusters);
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
            pending_extracts: Vec::new(),
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
                code_examples: vec![],
                source_ids: vec![],
                frequency: 1,
                skill_slug: None,
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

    #[test]
    fn test_default_max_consecutive_empty() {
        assert_eq!(DEFAULT_MAX_CONSECUTIVE_EMPTY, 2);
    }
}
