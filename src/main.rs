use anyhow::Result;
use clap::{Parser, Subcommand};
use skill_miner::{
    bundle, classifier, compressor, deployer, extractor, generator, graph, history, manifest, miner,
    parser, refiner, scorer, util, DraftStatus, MineConfig, PruneOptions,
};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "skill-miner")]
#[command(about = "Extract domain knowledge from Claude Code chat history and generate agent skills")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan conversation history and show statistics
    Scan {
        /// How many days back to scan
        #[arg(short, long, default_value = "30")]
        days: u32,
        /// Minimum messages per conversation
        #[arg(short, long, default_value = "4")]
        min_messages: usize,
        /// Fast mode: preview from history.jsonl without full conversation parse
        #[arg(short, long)]
        fast: bool,
        /// Filter by project path (substring match, for --fast mode)
        #[arg(short, long)]
        project: Option<String>,
    },

    /// Classify conversations by domain
    Classify {
        #[arg(short, long, default_value = "30")]
        days: u32,
        #[arg(short, long, default_value = "4")]
        min_messages: usize,
        /// Output JSON file for classifications
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Extract knowledge patterns from classified conversations
    Extract {
        /// Input JSON from classify step
        #[arg(short, long)]
        input: PathBuf,
        /// Output JSON file for patterns
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Maximum parallel AI calls
        #[arg(long, default_value = "4")]
        parallel: usize,
    },

    /// Generate skill drafts from extracted patterns
    Generate {
        /// Input JSON from extract step
        #[arg(short, long)]
        input: PathBuf,
        /// Output directory for skill drafts
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Run progressive mining: auto-expand time window until no new conversations
    Mine {
        /// Output directory for generated skills
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Dry run: show what would be generated without writing files
        #[arg(long)]
        dry_run: bool,
        /// Maximum parallel AI calls
        #[arg(long, default_value = "4")]
        parallel: usize,
        /// Maximum time windows to process (default: unlimited until no new convs)
        #[arg(long)]
        max_windows: Option<usize>,
        /// Maximum days to look back (default: 30)
        #[arg(long, default_value = "30")]
        max_days: u32,
        /// Minimum messages per conversation
        #[arg(short, long, default_value = "4")]
        min_messages: usize,
        /// Minimum significance ratio (0.0-1.0): stop if fewer than this fraction
        /// of conversations in a window are non-misc with confidence >= 0.5
        #[arg(long, default_value = "0.3")]
        min_significance: f64,
        /// Drafts directory
        #[arg(short, long)]
        dir: Option<PathBuf>,
        /// Auto-sync: commit and push drafts after mining
        #[arg(long)]
        sync: bool,
    },

    /// List skill drafts with their status
    List {
        /// Drafts directory
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Show diff between draft and deployed skill
    Diff {
        /// Skill slug name (omit for all)
        name: Option<String>,
        /// Drafts directory
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Approve skill drafts for deployment
    Approve {
        /// Skill slugs to approve
        names: Vec<String>,
        /// Approve all drafts
        #[arg(long)]
        all: bool,
        /// Drafts directory
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Reject skill drafts
    Reject {
        /// Skill slugs to reject
        names: Vec<String>,
        /// Drafts directory
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Deploy approved skills to ~/.claude/skills/
    Deploy {
        /// Deploy specific skills by name (or use --approved)
        names: Vec<String>,
        /// Deploy all approved drafts
        #[arg(long)]
        approved: bool,
        /// Drafts directory
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Remove low-quality or duplicate drafts
    Prune {
        /// Remove "misc" domain drafts
        #[arg(long)]
        misc: bool,
        /// Remove rejected drafts
        #[arg(long)]
        rejected: bool,
        /// Remove Japanese-named duplicates
        #[arg(long)]
        duplicates: bool,
        /// Drafts directory
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Export skills as a portable .skillpack bundle
    Export {
        /// Output directory for the bundle
        output: PathBuf,
        /// Bundle name
        #[arg(long, default_value = "my-skills")]
        name: String,
        /// Bundle author
        #[arg(long)]
        author: Option<String>,
        /// Bundle description
        #[arg(long, default_value = "Exported skill bundle")]
        description: String,
        /// Only export approved/deployed skills
        #[arg(long)]
        approved_only: bool,
        /// Include referenced memory/context files in bundle
        #[arg(long)]
        include_context: bool,
        /// Export a sanitized public bundle
        #[arg(long, conflicts_with = "both")]
        public: bool,
        /// Export both private and sanitized public bundles
        #[arg(long, conflicts_with = "public")]
        both: bool,
        /// Explicit output directory for the public bundle (used with --both)
        #[arg(long, requires = "both")]
        public_output: Option<PathBuf>,
        /// Drafts directory
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Import skills from a .skillpack bundle
    Import {
        /// Path to the .skillpack directory
        bundle_path: PathBuf,
        /// Drafts directory to import into
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Verify integrity of a .skillpack bundle
    Verify {
        /// Path to the .skillpack directory
        bundle_path: PathBuf,
    },

    /// Validate bundle structure and content quality
    Validate {
        /// Path to the .skillpack directory
        bundle_path: PathBuf,
        /// Apply stricter checks for public sharing
        #[arg(long)]
        public: bool,
        /// Auto-fix common structural issues before validating
        #[arg(long)]
        fix: bool,
    },

    /// Show dependency graph between skills, memory, and CLAUDE.md files
    Graph {
        /// Skills directory to scan
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Consolidate: score skills from invocation logs and rebuild descriptions
    Consolidate {
        /// Skill slugs to consolidate (or use --all)
        names: Vec<String>,
        /// Consolidate all skills
        #[arg(long)]
        all: bool,
        /// How many days of invocation logs to scan
        #[arg(long, default_value = "30")]
        days: u32,
        /// Minimum score threshold; skills below this get rejected
        #[arg(long, default_value = "0.1")]
        min_score: f64,
        /// Show changes without writing
        #[arg(long)]
        dry_run: bool,
        /// Use AI to refine descriptions based on actual trigger phrases
        #[arg(long)]
        refine: bool,
        /// Drafts directory
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Show today's work timeline from history.jsonl
    Today {
        /// Filter by project path (substring match)
        #[arg(short, long)]
        project: Option<String>,
        /// Search display text (substring match, case-insensitive)
        #[arg(short, long)]
        search: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = MineConfig::default();

    match cli.command {
        Command::Scan { days, min_messages, fast, project } => {
            if fast {
                cmd_scan_fast(&config, days, project)
            } else {
                cmd_scan(&config, days, min_messages)
            }
        }
        Command::Classify {
            days,
            min_messages,
            output,
        } => cmd_classify(&config, days, min_messages, output),
        Command::Extract { input, output, parallel } => cmd_extract(&config, input, output, parallel),
        Command::Generate { input, output } => cmd_generate(&config, input, output),
        Command::Mine {
            output,
            dry_run,
            parallel,
            max_windows,
            max_days,
            min_messages,
            min_significance,
            dir,
            sync,
        } => cmd_mine(&config, output, dry_run, parallel, max_windows, max_days, min_messages, min_significance, dir, sync),
        Command::List { dir } => cmd_list(&config, dir),
        Command::Diff { name, dir } => cmd_diff(&config, name, dir),
        Command::Approve { names, all, dir } => cmd_approve(&config, names, all, dir),
        Command::Reject { names, dir } => cmd_reject(&config, names, dir),
        Command::Deploy {
            names,
            approved,
            dir,
        } => cmd_deploy(&config, names, approved, dir),
        Command::Prune {
            misc,
            rejected,
            duplicates,
            dir,
        } => cmd_prune(&config, misc, rejected, duplicates, dir),
        Command::Export {
            output,
            name,
            author,
            description,
            approved_only,
            include_context,
            public,
            both,
            public_output,
            dir,
        } => cmd_export(
            &config,
            output,
            name,
            author,
            description,
            approved_only,
            include_context,
            public,
            both,
            public_output,
            dir,
        ),
        Command::Graph { dir } => cmd_graph(&config, dir),
        Command::Import { bundle_path, dir } => cmd_import(&config, bundle_path, dir),
        Command::Verify { bundle_path } => cmd_verify(bundle_path),
        Command::Validate {
            bundle_path,
            public,
            fix,
        } => cmd_validate(bundle_path, public, fix),
        Command::Consolidate {
            names,
            all,
            days,
            min_score,
            dry_run,
            refine,
            dir,
        } => cmd_consolidate(&config, names, all, days, min_score, dry_run, refine, dir),
        Command::Today { project, search } => cmd_today(&config, project, search),
    }
}

fn cmd_scan(config: &MineConfig, days: u32, min_messages: usize) -> Result<()> {
    eprintln!("Scanning conversations (last {} days)...", days);

    let conversations = parser::parse_all(&config.projects_dir, min_messages, days)?;

    eprintln!("Found {} conversations (>= {} messages)\n", conversations.len(), min_messages);

    // Show statistics
    let total_messages: usize = conversations.iter().map(|c| c.message_count()).sum();
    let total_size: u64 = conversations
        .iter()
        .filter_map(|c| std::fs::metadata(&c.source_path).ok())
        .map(|m| m.len())
        .sum();

    println!("=== Scan Results ===");
    println!("Conversations: {}", conversations.len());
    println!("Total messages: {}", total_messages);
    println!("Total size: {:.1} MB", total_size as f64 / 1_048_576.0);
    println!();

    // Show summaries
    let summaries = compressor::compress_all(&conversations);
    for s in summaries.iter().take(20) {
        println!(
            "[{}] msgs={:3} topics=[{}]",
            &s.id[..8.min(s.id.len())],
            s.message_count,
            s.topics.join(", ")
        );
        println!("  {}", util::truncate(&s.first_message, 80));
        println!();
    }

    if summaries.len() > 20 {
        println!("... and {} more", summaries.len() - 20);
    }

    Ok(())
}

fn cmd_scan_fast(config: &MineConfig, days: u32, project: Option<String>) -> Result<()> {
    eprintln!("Fast scan from history.jsonl (last {} days)...", days);

    let entries = history::parse_history(&config.history_path)?;
    let filtered = history::filter_by_days(&entries, days);

    let filtered: Vec<_> = if let Some(ref proj) = project {
        history::filter_by_project(
            &filtered.into_iter().cloned().collect::<Vec<_>>(),
            proj,
        )
        .into_iter()
        .cloned()
        .collect()
    } else {
        filtered.into_iter().cloned().collect()
    };

    println!("=== Fast Scan (history.jsonl) ===");
    println!("Entries: {}", filtered.len());
    if let Some(ref proj) = project {
        println!("Project filter: {}", proj);
    }
    println!();

    // Group by project
    let mut by_project: std::collections::HashMap<String, Vec<&history::HistoryEntry>> =
        std::collections::HashMap::new();
    for entry in &filtered {
        by_project
            .entry(entry.project.clone())
            .or_default()
            .push(entry);
    }

    let mut projects: Vec<_> = by_project.iter().collect();
    projects.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    for (proj, entries) in projects.iter().take(20) {
        println!("[{}] {} entries", proj, entries.len());
        // Show latest 3 entries
        let mut sorted: Vec<_> = entries.iter().copied().collect();
        sorted.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        for e in sorted.iter().take(3) {
            let display = util::truncate(&e.display, 80);
            println!("  {}", display);
        }
        println!();
    }

    if projects.len() > 20 {
        println!("... and {} more projects", projects.len() - 20);
    }

    Ok(())
}

fn cmd_today(config: &MineConfig, project: Option<String>, search: Option<String>) -> Result<()> {
    let entries = history::parse_history(&config.history_path)?;
    let today_entries = history::filter_today(&entries);

    // Apply project filter
    let today_entries: Vec<_> = if let Some(ref proj) = project {
        let proj_lower = proj.to_lowercase();
        today_entries
            .into_iter()
            .filter(|e| e.project.to_lowercase().contains(&proj_lower))
            .collect()
    } else {
        today_entries
    };

    // Apply search filter
    let today_entries: Vec<_> = if let Some(ref query) = search {
        let query_lower = query.to_lowercase();
        today_entries
            .into_iter()
            .filter(|e| e.display.to_lowercase().contains(&query_lower))
            .collect()
    } else {
        today_entries
    };

    // Count unique projects
    let mut project_set: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for e in &today_entries {
        if !e.project.is_empty() {
            project_set.insert(&e.project);
        }
    }

    let today_str = chrono::Local::now().format("%Y-%m-%d");
    println!("=== Today's Activity ({}) ===", today_str);
    println!("{} sessions | {} projects", today_entries.len(), project_set.len());
    if let Some(ref proj) = project {
        println!("Project filter: {}", proj);
    }
    if let Some(ref query) = search {
        println!("Search: {}", query);
    }
    println!();

    for entry in &today_entries {
        // Format timestamp as HH:MM local time
        let dt = chrono::DateTime::from_timestamp_millis(entry.timestamp as i64)
            .unwrap_or_default()
            .with_timezone(&chrono::Local);
        let time_str = dt.format("%H:%M");

        // Extract last path component for project name
        let proj_name = entry
            .project
            .rsplit(|c| c == '\\' || c == '/')
            .next()
            .unwrap_or(&entry.project);

        let display = util::truncate(&entry.display, 80);
        println!("{}  [{}] {}", time_str, proj_name, display);
    }

    Ok(())
}

fn cmd_classify(
    config: &MineConfig,
    days: u32,
    min_messages: usize,
    output: Option<PathBuf>,
) -> Result<()> {
    eprintln!("Parsing conversations (last {} days)...", days);
    let conversations = parser::parse_all(&config.projects_dir, min_messages, days)?;
    eprintln!("Found {} conversations", conversations.len());

    eprintln!("Compressing...");
    let summaries = compressor::compress_all(&conversations);

    eprintln!("Classifying with AI...");
    let classified = classifier::classify(&summaries, &config.ai_options)?;

    // Show results
    let groups = classifier::group_by_domain(&classified);
    println!("=== Classification Results ===\n");
    let mut domains: Vec<_> = groups.iter().collect();
    domains.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    for (domain, convs) in &domains {
        println!("{}: {} conversations", domain, convs.len());
    }

    // Save if output specified
    if let Some(path) = output {
        let json = serde_json::to_string_pretty(&classified)?;
        std::fs::write(&path, json)?;
        eprintln!("\nSaved to {}", path.display());
    }

    Ok(())
}

fn cmd_extract(config: &MineConfig, input: PathBuf, output: Option<PathBuf>, parallel: usize) -> Result<()> {
    let json = std::fs::read_to_string(&input)?;
    let classified: Vec<skill_miner::ClassifiedConversation> = serde_json::from_str(&json)?;

    let groups = classifier::group_by_domain(&classified);

    eprintln!("Extracting patterns from {} domains (parallel, max {})...", groups.len(), parallel);

    // Standalone extract: no pre-parsed conversations, will parse from source_path
    let (clusters, _extract_calls, failed_domains) =
        extractor::extract_all_parallel(&groups, None, &config.ai_options, parallel)?;
    if !failed_domains.is_empty() {
        eprintln!("  {} domain(s) failed, {} succeeded", failed_domains.len(), clusters.len());
    }

    for cluster in &clusters {
        println!(
            "  {} → {} patterns",
            cluster.domain,
            cluster.patterns.len()
        );
    }

    if let Some(path) = output {
        let json = serde_json::to_string_pretty(&clusters)?;
        std::fs::write(&path, json)?;
        eprintln!("\nSaved to {}", path.display());
    }

    Ok(())
}

fn cmd_generate(config: &MineConfig, input: PathBuf, output: Option<PathBuf>) -> Result<()> {
    let json = std::fs::read_to_string(&input)?;
    let clusters: Vec<skill_miner::DomainCluster> = serde_json::from_str(&json)?;

    let mut drafts = generator::generate_skills(&clusters);

    // Check against existing skills
    generator::check_existing_skills(&mut drafts, &config.skills_dir)?;

    let out_dir = output.unwrap_or_else(|| PathBuf::from("./skill-drafts"));
    std::fs::create_dir_all(&out_dir)?;

    for draft in &drafts {
        let status = if draft.existing_skill.is_some() {
            "UPDATE"
        } else {
            "NEW"
        };

        if let Some(ref diff) = draft.diff {
            let (added, removed) = generator::parse_diff_summary(diff);
            println!(
                "[{}] {}: +{} lines, -{} lines",
                status, draft.name, added, removed
            );
            for line in diff.lines() {
                println!("  {}", line);
            }
        } else {
            println!("[{}] {}: {}", status, draft.name, util::truncate(&draft.description, 80));
        }

        let content = generator::format_skill_md(draft);
        let path = out_dir.join(format!("{}.md", draft.name));
        std::fs::write(&path, content)?;
    }

    eprintln!("\nGenerated {} skill drafts in {}", drafts.len(), out_dir.display());

    Ok(())
}

fn cmd_mine(
    config: &MineConfig,
    output: Option<PathBuf>,
    dry_run: bool,
    parallel: usize,
    max_windows: Option<usize>,
    max_days: u32,
    min_messages: usize,
    min_significance: f64,
    dir: Option<PathBuf>,
    sync: bool,
) -> Result<()> {
    let drafts_dir = output.unwrap_or_else(|| {
        dir.unwrap_or_else(|| config.skills_dir.join("drafts"))
    });
    std::fs::create_dir_all(&drafts_dir)?;

    let mut mf = load_or_create_manifest(&drafts_dir)?;

    let progressive = miner::ProgressiveConfig {
        max_days,
        max_windows,
        min_messages,
        parallel,
        min_significance_ratio: min_significance,
    };

    let result = miner::mine_progressive(config, &mut mf, &progressive, dry_run, &drafts_dir)?;

    // Display results
    for draft in &result.drafts {
        let status = if draft.existing_skill.is_some() {
            "UPDATE"
        } else {
            "NEW"
        };
        if let Some(ref diff) = draft.diff {
            let (added, removed) = generator::parse_diff_summary(diff);
            println!("[{}] {}: +{} lines, -{} lines", status, draft.name, added, removed);
        } else {
            println!("[{}] {}", status, draft.name);
        }
    }

    if !dry_run {
        // Deploy directly to skills dir (no draft stage)
        std::fs::create_dir_all(&config.skills_dir)?;
        for draft in &result.drafts {
            let content = generator::format_skill_md(draft);
            let path = config.skills_dir.join(format!("{}.md", draft.name));
            std::fs::write(&path, content)?;
        }

        // Merge into manifest and mark as deployed
        miner::merge_into_manifest(&mut mf, &result.drafts, &result.clusters);
        let now = chrono::Utc::now();
        for draft in &result.drafts {
            if let Some(entry) = mf.entries.iter_mut().find(|e| e.slug == draft.name) {
                entry.status = skill_miner::DraftStatus::Deployed;
                entry.deployed_at = Some(now);
            }
        }
        manifest::write_manifest(&drafts_dir, &mf)?;

        eprintln!(
            "\nDeployed {} skills to {}",
            result.drafts.len(),
            config.skills_dir.display()
        );

        // Auto-sync if requested
        if sync {
            let new_count = result.drafts.iter().filter(|d| d.existing_skill.is_none()).count();
            let updated_count = result.drafts.iter().filter(|d| d.existing_skill.is_some()).count();
            let sync_config = skill_miner::sync::SyncConfig {
                drafts_dir: config.skills_dir.clone(),
                remote: "origin".to_string(),
                branch: "main".to_string(),
            };
            let sync_result = skill_miner::sync::sync_drafts(&sync_config, new_count, updated_count);
            if sync_result.committed {
                eprintln!(
                    "[sync] committed {} files: {}",
                    sync_result.files_changed, sync_result.commit_message
                );
                if sync_result.pushed {
                    eprintln!("[sync] pushed successfully");
                }
            }
        }
    } else {
        eprintln!("\nDry run: {} drafts would be generated", result.drafts.len());
    }

    // Stats summary
    eprintln!();
    eprintln!("=== Progressive Mine Results ===");
    eprintln!("Windows processed: {}", result.windows_processed);
    eprintln!("New conversations: {}", result.new_conversations);
    if result.skipped_low_value > 0 {
        eprintln!("Low-value windows: {}", result.skipped_low_value);
    }
    eprintln!(
        "Classify: {} AI calls",
        result.stats.classify_calls
    );
    if result.stats.extract_failures > 0 {
        eprintln!(
            "Extract: {} AI calls ({} ok, {} failed → will retry next run)",
            result.stats.extract_calls,
            result.stats.extract_calls - result.stats.extract_failures,
            result.stats.extract_failures
        );
    } else {
        eprintln!(
            "Extract: {} AI calls ({} domains)",
            result.stats.extract_calls,
            result.stats.extract_calls
        );
    }
    eprintln!("Total: {} AI calls", result.stats.total_calls);

    if !mf.pending_extracts.is_empty() {
        eprintln!("Pending: {} conversations awaiting retry next run", mf.pending_extracts.len());
    }

    // Tool coverage check: report projects referenced in conversations but lacking skills
    let all_files: Vec<Vec<String>> = result
        .clusters
        .iter()
        .flat_map(|c| c.conversations.iter())
        .map(|c| c.summary.files_touched.clone())
        .collect();

    if !all_files.is_empty() {
        let home_dir = skill_miner::util::home_dir();
        let uncovered =
            skill_miner::tool_coverage::find_uncovered_projects(&all_files, &config.skills_dir, &home_dir);
        let report = skill_miner::tool_coverage::format_report(&uncovered);
        if !report.is_empty() {
            eprint!("{}", report);
        }
    }

    Ok(())
}

// ── State management commands ──

fn resolve_drafts_dir(_config: &MineConfig, dir: Option<PathBuf>) -> PathBuf {
    dir.unwrap_or_else(|| PathBuf::from("./skill-drafts"))
}

fn load_or_create_manifest(dir: &std::path::Path) -> Result<skill_miner::Manifest> {
    match manifest::read_manifest(dir) {
        Ok(m) => Ok(m),
        Err(_) => {
            eprintln!("No manifest.toml found, scanning .md files...");
            let m = manifest::create_from_directory(dir)?;
            manifest::write_manifest(dir, &m)?;
            Ok(m)
        }
    }
}

fn cmd_list(config: &MineConfig, dir: Option<PathBuf>) -> Result<()> {
    let drafts_dir = resolve_drafts_dir(config, dir);
    let mf = load_or_create_manifest(&drafts_dir)?;

    println!(
        "=== Skill Drafts ({} total) ===\n",
        mf.entries.len()
    );

    // Sort: deployed first, then approved, then draft, then rejected
    let mut entries: Vec<_> = mf.entries.iter().collect();
    entries.sort_by_key(|e| match e.status {
        DraftStatus::Deployed => 0,
        DraftStatus::Approved => 1,
        DraftStatus::Draft => 2,
        DraftStatus::Rejected => 3,
    });

    for e in &entries {
        let deployed_info = if let Some(dt) = e.deployed_at {
            format!("  deployed: {}", dt.format("%Y-%m-%d"))
        } else {
            String::new()
        };
        let score_info = match e.score {
            Some(s) => format!("  score: {:.2}", s),
            None => String::new(),
        };
        let fire_info = match e.fire_count {
            Some(f) => format!("  fires: {}", f),
            None => String::new(),
        };
        println!(
            "[{:9}] {:<20} {:<12} {} patterns{}{}{}",
            e.status.to_string(),
            e.slug,
            e.domain,
            e.pattern_count,
            score_info,
            fire_info,
            deployed_info
        );
    }

    Ok(())
}

fn cmd_diff(config: &MineConfig, name: Option<String>, dir: Option<PathBuf>) -> Result<()> {
    let drafts_dir = resolve_drafts_dir(config, dir);

    if let Some(slug) = name {
        let result = deployer::diff_skill(&drafts_dir, &config.skills_dir, &slug)?;
        println!("{}", result);
    } else {
        // Diff all
        let mf = load_or_create_manifest(&drafts_dir)?;
        for entry in &mf.entries {
            let result =
                deployer::diff_skill(&drafts_dir, &config.skills_dir, &entry.slug)?;
            println!("{}", result);
        }
    }

    Ok(())
}

fn cmd_approve(
    config: &MineConfig,
    names: Vec<String>,
    all: bool,
    dir: Option<PathBuf>,
) -> Result<()> {
    let drafts_dir = resolve_drafts_dir(config, dir);
    let mut mf = load_or_create_manifest(&drafts_dir)?;

    let slugs: Vec<String> = if all {
        mf.entries
            .iter()
            .filter(|e| e.status == DraftStatus::Draft)
            .map(|e| e.slug.clone())
            .collect()
    } else {
        names
    };

    for slug in &slugs {
        match manifest::update_status(&mut mf, slug, DraftStatus::Approved) {
            Ok(()) => println!("[approved] {}", slug),
            Err(e) => eprintln!("  skip {}: {}", slug, e),
        }
    }

    manifest::write_manifest(&drafts_dir, &mf)?;
    eprintln!("\nApproved {} drafts", slugs.len());

    Ok(())
}

fn cmd_reject(config: &MineConfig, names: Vec<String>, dir: Option<PathBuf>) -> Result<()> {
    let drafts_dir = resolve_drafts_dir(config, dir);
    let mut mf = load_or_create_manifest(&drafts_dir)?;

    for slug in &names {
        match manifest::update_status(&mut mf, slug, DraftStatus::Rejected) {
            Ok(()) => println!("[rejected] {}", slug),
            Err(e) => eprintln!("  skip {}: {}", slug, e),
        }
    }

    manifest::write_manifest(&drafts_dir, &mf)?;
    Ok(())
}

fn cmd_deploy(
    config: &MineConfig,
    names: Vec<String>,
    approved: bool,
    dir: Option<PathBuf>,
) -> Result<()> {
    let drafts_dir = resolve_drafts_dir(config, dir);
    let mut mf = load_or_create_manifest(&drafts_dir)?;

    let results = if approved {
        deployer::deploy_approved(&drafts_dir, &config.skills_dir, &mut mf)?
    } else if !names.is_empty() {
        deployer::deploy_by_names(&drafts_dir, &config.skills_dir, &mut mf, &names)?
    } else {
        eprintln!("Specify skill names or use --approved");
        return Ok(());
    };

    for r in &results {
        let action = if r.was_update { "updated" } else { "created" };
        println!("[{}] {} → {}", action, r.slug, r.target_path.display());
    }

    manifest::write_manifest(&drafts_dir, &mf)?;
    eprintln!("\nDeployed {} skills to {}", results.len(), config.skills_dir.display());

    Ok(())
}

fn cmd_prune(
    config: &MineConfig,
    misc: bool,
    rejected: bool,
    duplicates: bool,
    dir: Option<PathBuf>,
) -> Result<()> {
    let drafts_dir = resolve_drafts_dir(config, dir);
    let mut mf = load_or_create_manifest(&drafts_dir)?;

    let opts = PruneOptions {
        misc,
        rejected,
        duplicates,
    };

    if !misc && !rejected && !duplicates {
        eprintln!("Specify at least one prune option: --misc, --rejected, --duplicates");
        return Ok(());
    }

    let removed = deployer::prune(&drafts_dir, &mut mf, &opts)?;

    for slug in &removed {
        println!("[removed] {}", slug);
    }

    manifest::write_manifest(&drafts_dir, &mf)?;
    eprintln!("\nPruned {} drafts", removed.len());

    Ok(())
}

fn cmd_graph(config: &MineConfig, dir: Option<PathBuf>) -> Result<()> {
    let skills_dir = dir.unwrap_or_else(|| config.skills_dir.clone());
    let home = util::home_dir();

    // Collect memory dirs from all projects
    let mut memory_dirs: Vec<PathBuf> = Vec::new();
    let projects_dir = &config.projects_dir;
    if projects_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(projects_dir) {
            for entry in entries.flatten() {
                let mem_dir = entry.path().join("memory");
                if mem_dir.is_dir() {
                    memory_dirs.push(mem_dir);
                }
            }
        }
    }

    // Collect CLAUDE.md paths
    let claude_md_paths: Vec<PathBuf> = [
        home.join("CLAUDE.md"),
        home.join(".claude").join("CLAUDE.md"),
    ]
    .into_iter()
    .filter(|p| p.exists())
    .collect();

    let dep_graph = graph::build_graph(&skills_dir, &memory_dirs, &claude_md_paths);

    println!("=== Dependency Graph ===\n");

    for node in &dep_graph.nodes {
        println!("{}", node.path);
        for dep in &node.outgoing {
            let broken = dep_graph
                .broken_links
                .iter()
                .any(|b| b.from == dep.from && b.to == dep.to);
            let suffix = if broken { " [BROKEN]" } else { "" };
            println!(
                "  \u{2192} [{:?}] {} (line {}){}",
                dep.dep_type, dep.to, dep.line, suffix
            );
        }
        for dep in &node.incoming {
            println!(
                "  \u{2190} [{:?}] from {} (line {})",
                dep.dep_type, dep.from, dep.line
            );
        }
        println!();
    }

    // Summary
    let total_refs: usize = dep_graph.nodes.iter().map(|n| n.outgoing.len()).sum();
    println!("=== Summary ===");
    println!("Files scanned: {}", dep_graph.nodes.len());
    println!("References found: {}", total_refs);
    println!(
        "Broken links: {}{}",
        dep_graph.broken_links.len(),
        if dep_graph.broken_links.is_empty() {
            String::new()
        } else {
            format!(
                " ({})",
                dep_graph
                    .broken_links
                    .iter()
                    .map(|b| b.to.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );
    println!(
        "Orphan files: {}{}",
        dep_graph.orphans.len(),
        if dep_graph.orphans.is_empty() {
            String::new()
        } else {
            format!(
                " ({})",
                dep_graph
                    .orphans
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );

    Ok(())
}

fn cmd_export(
    config: &MineConfig,
    output: PathBuf,
    name: String,
    author: Option<String>,
    description: String,
    approved_only: bool,
    include_context: bool,
    public: bool,
    both: bool,
    public_output: Option<PathBuf>,
    dir: Option<PathBuf>,
) -> Result<()> {
    let drafts_dir = resolve_drafts_dir(config, dir);
    let mf = load_or_create_manifest(&drafts_dir)?;

    let private_opts = bundle::ExportOptions {
        approved_only,
        name: name.clone(),
        author: author.clone(),
        description: description.clone(),
        include_context,
        public_sanitized: false,
    };

    let public_opts = bundle::ExportOptions {
        approved_only,
        name,
        author,
        description,
        include_context,
        public_sanitized: true,
    };

    if both {
        let private_bundle = bundle::export_bundle(&drafts_dir, &output, &mf, &private_opts)?;
        print_bundle_summary("Private", &output, &private_bundle, include_context);

        let public_dir = public_output.unwrap_or_else(|| derive_public_output_path(&output));
        let public_bundle = bundle::export_bundle(&drafts_dir, &public_dir, &mf, &public_opts)?;
        print_bundle_summary("Public (sanitized)", &public_dir, &public_bundle, include_context);

        return Ok(());
    }

    let opts = if public { &public_opts } else { &private_opts };
    let label = if public { "Public (sanitized)" } else { "Private" };
    let bun = bundle::export_bundle(&drafts_dir, &output, &mf, opts)?;
    print_bundle_summary(label, &output, &bun, include_context);

    Ok(())
}

fn derive_public_output_path(output: &std::path::Path) -> PathBuf {
    let output_str = output.to_string_lossy();
    if output_str.ends_with(".skillpack") {
        PathBuf::from(output_str.replace(".skillpack", "-public.skillpack"))
    } else {
        PathBuf::from(format!("{}-public", output_str))
    }
}

fn print_bundle_summary(
    label: &str,
    output: &std::path::Path,
    bun: &skill_miner::SkillBundle,
    include_context: bool,
) {
    println!("=== Exported Bundle ({}) ===", label);
    println!("Name: {}", bun.name);
    println!("Skills: {}", bun.skills.len());
    println!(
        "Source: {} conversations, {} domains, {} patterns",
        bun.source.conversations, bun.source.domains, bun.source.patterns
    );
    if include_context {
        let ctx_dir = output.join("context").join("memory");
        if ctx_dir.exists() {
            let count = std::fs::read_dir(&ctx_dir).map(|rd| rd.count()).unwrap_or(0);
            println!("Context files: {}", count);
        }
    }
    println!("Output: {}", output.display());
}

fn cmd_import(config: &MineConfig, bundle_path: PathBuf, dir: Option<PathBuf>) -> Result<()> {
    let drafts_dir = resolve_drafts_dir(config, dir);
    let mut mf = load_or_create_manifest(&drafts_dir)?;

    let mut result = bundle::import_bundle(&bundle_path, &drafts_dir, &mut mf)?;

    if !result.imported.is_empty() {
        println!("Imported:");
        for slug in &result.imported {
            println!("  + {}", slug);
        }
    }
    if !result.skipped.is_empty() {
        println!("Skipped (identical):");
        for slug in &result.skipped {
            println!("  = {}", slug);
        }
    }
    if !result.conflicted.is_empty() {
        println!("Conflicted (saved as .imported.md):");
        for slug in &result.conflicted {
            println!("  ! {}", slug);
        }
    }

    // Restore context files if present
    let ctx_dir = bundle_path.join("context").join("memory");
    if ctx_dir.exists() {
        // Find current project's memory dir by matching CWD against existing project dirs
        let projects_dir = config.projects_dir.clone();
        let memory_dir = if projects_dir.exists() {
            let cwd = std::env::current_dir().unwrap_or_default();
            let cwd_str = cwd.to_string_lossy().replace(['/', '\\', ':'], "-");

            // Find the best matching existing project directory
            let mut best_match: Option<PathBuf> = None;
            let mut best_len = 0;
            if let Ok(entries) = std::fs::read_dir(&projects_dir) {
                for entry in entries.flatten() {
                    let dir_name = entry.file_name().to_string_lossy().to_string();
                    // Check if CWD key starts with or contains this project key
                    if cwd_str.starts_with(&dir_name) && dir_name.len() > best_len {
                        best_len = dir_name.len();
                        best_match = Some(entry.path());
                    }
                }
            }

            let project_dir = best_match.unwrap_or_else(|| {
                // No match found; use the CWD-derived key
                projects_dir.join(&cwd_str)
            });
            let mem = project_dir.join("memory");
            std::fs::create_dir_all(&mem)?;
            mem
        } else {
            // Fallback: create memory dir next to drafts
            let mem = drafts_dir.join("imported-context");
            std::fs::create_dir_all(&mem)?;
            mem
        };

        bundle::import_context(&bundle_path, &memory_dir, &mut result)?;

        if !result.context_imported.is_empty() {
            println!("Context imported:");
            for f in &result.context_imported {
                println!("  + {}", f);
            }
        }
        if !result.context_conflicted.is_empty() {
            println!("Context conflicted (saved as .imported.md):");
            for f in &result.context_conflicted {
                println!("  ! {}", f);
            }
        }
    }

    manifest::write_manifest(&drafts_dir, &mf)?;

    eprintln!(
        "\nImport: {} new, {} skipped, {} conflicts",
        result.imported.len(),
        result.skipped.len(),
        result.conflicted.len()
    );
    if !result.context_imported.is_empty() || !result.context_conflicted.is_empty() {
        eprintln!(
            "Context: {} imported, {} conflicts",
            result.context_imported.len(),
            result.context_conflicted.len()
        );
    }

    Ok(())
}

fn cmd_consolidate(
    config: &MineConfig,
    names: Vec<String>,
    all: bool,
    days: u32,
    min_score: f64,
    dry_run: bool,
    refine: bool,
    dir: Option<PathBuf>,
) -> Result<()> {
    let drafts_dir = resolve_drafts_dir(config, dir);
    let mut mf = load_or_create_manifest(&drafts_dir)?;

    if mf.entries.is_empty() {
        eprintln!("No drafts found in {}", drafts_dir.display());
        return Ok(());
    }

    // Step 1: Parse chat history and extract skill invocations
    eprintln!("[1/3] Scanning chat history (last {} days) for skill invocations...", days);
    let conversations = parser::parse_all(&config.projects_dir, 1, days)?;
    let invocations = parser::extract_skill_invocations(&conversations);
    eprintln!("  → {} conversations, {} skill invocations", conversations.len(), invocations.len());

    // Step 2: Score skills
    eprintln!("[2/3] Scoring skills...");

    // Build minimal clusters from manifest entries for scoring
    let clusters: Vec<skill_miner::DomainCluster> = mf
        .entries
        .iter()
        .map(|e| {
            let patterns: Vec<skill_miner::KnowledgePattern> = (0..e.pattern_count)
                .map(|_| skill_miner::KnowledgePattern {
                    title: String::new(),
                    description: String::new(),
                    steps: vec![],
                    code_examples: vec![],
                    source_ids: vec![],
                    frequency: 1,
                    skill_slug: None,
                })
                .collect();
            skill_miner::DomainCluster {
                domain: e.domain.clone(),
                conversations: vec![],
                patterns,
            }
        })
        .collect();

    let scores = scorer::score_skills(&invocations, &mf, &clusters);

    // Build score lookup
    let score_map: HashMap<String, f64> = scores.iter().cloned().collect();

    // Build fire count and productive count lookups
    let mut fire_map: HashMap<&str, usize> = HashMap::new();
    let mut productive_map: HashMap<&str, usize> = HashMap::new();
    for inv in &invocations {
        *fire_map.entry(inv.skill_name.as_str()).or_insert(0) += 1;
        if inv.was_productive {
            *productive_map.entry(inv.skill_name.as_str()).or_insert(0) += 1;
        }
    }

    // Build trigger_context lookup: slug -> Vec<String>
    let mut trigger_map: HashMap<&str, Vec<String>> = HashMap::new();
    for inv in &invocations {
        if let Some(ref ctx) = inv.trigger_context {
            trigger_map
                .entry(inv.skill_name.as_str())
                .or_default()
                .push(ctx.clone());
        }
    }

    // Filter to requested slugs
    let target_slugs: Vec<String> = if all {
        mf.entries.iter().map(|e| e.slug.clone()).collect()
    } else if !names.is_empty() {
        names
    } else {
        eprintln!("Specify skill names or use --all");
        return Ok(());
    };

    // Step 3: Display and apply
    eprintln!("[3/3] Results:\n");
    println!("{:<20} {:>6} {:>5}  {}", "SKILL", "SCORE", "FIRES", "STATUS");
    println!("{}", "-".repeat(50));

    let mut rejected_count = 0;
    let mut updated_count = 0;

    for slug in &target_slugs {
        let score = score_map.get(slug).copied().unwrap_or(0.0);
        let fires = fire_map.get(slug.as_str()).copied().unwrap_or(0);

        let status_change = if score < min_score {
            "→ rejected"
        } else {
            ""
        };

        println!(
            "{:<20} {:>6.3} {:>5}  {}",
            slug, score, fires, status_change
        );

        if !dry_run {
            if let Some(entry) = manifest::find_entry_mut(&mut mf, slug) {
                entry.score = Some(score);
                entry.fire_count = Some(fires);
                updated_count += 1;

                if score < min_score && entry.status != DraftStatus::Rejected {
                    match entry.status {
                        DraftStatus::Draft => {
                            entry.status = DraftStatus::Rejected;
                            rejected_count += 1;
                        }
                        DraftStatus::Approved | DraftStatus::Deployed => {
                            entry.status = DraftStatus::Rejected;
                            rejected_count += 1;
                        }
                        DraftStatus::Rejected => {}
                    }
                }
            }
        }
    }

    // === Fire Diagnostics ===
    println!();
    println!("=== Fire Diagnostics ===");
    println!();

    // UNDER-TRIGGER: fire_count==0 && deployed > 14 days
    let under_triggered: Vec<_> = target_slugs
        .iter()
        .filter_map(|slug| {
            let entry = manifest::find_entry(&mf, slug)?;
            let fires = fire_map.get(slug.as_str()).copied().unwrap_or(0);
            if fires > 0 {
                return None;
            }
            let deployed_at = entry.deployed_at?;
            let days_since = (chrono::Utc::now() - deployed_at).num_days();
            if days_since > 14 {
                Some((slug.clone(), deployed_at, days_since))
            } else {
                None
            }
        })
        .collect();

    if !under_triggered.is_empty() {
        println!("[UNDER-TRIGGER] The following skills have zero fires after 14+ days deployed:");
        for (slug, deployed_at, _days) in &under_triggered {
            println!("  {} (deployed {}, 0 fires)", slug, deployed_at.format("%Y-%m-%d"));
        }
        println!("  -> Possible insufficient trigger phrases in description. Try --refine");
        println!();
    }

    // LOW-PRODUCTIVE: productive_rate < 0.5 && fire_count >= 3
    let low_productive: Vec<_> = target_slugs
        .iter()
        .filter_map(|slug| {
            let fires = fire_map.get(slug.as_str()).copied().unwrap_or(0);
            if fires < 3 {
                return None;
            }
            let productive = productive_map.get(slug.as_str()).copied().unwrap_or(0);
            let rate = productive as f64 / fires as f64;
            if rate < 0.5 {
                Some((slug.clone(), fires, productive, rate))
            } else {
                None
            }
        })
        .collect();

    if !low_productive.is_empty() {
        println!("[LOW-PRODUCTIVE] The following skills fire but have low productive rate (< 50%):");
        for (slug, fires, productive, rate) in &low_productive {
            println!(
                "  {}: {} fires, {} productive ({:.0}%)",
                slug,
                fires,
                productive,
                rate * 100.0
            );
        }
        println!("  -> Possible over-triggering. Consider narrowing description scope");
        println!();
    }

    if under_triggered.is_empty() && low_productive.is_empty() {
        println!("No issues found.");
        println!();
    }

    // === --refine: description refinement ===
    if refine {
        eprintln!("=== Description Refinement ===\n");

        let refine_targets: Vec<_> = target_slugs
            .iter()
            .filter(|slug| {
                let fires = fire_map.get(slug.as_str()).copied().unwrap_or(0);
                fires > 0 && trigger_map.contains_key(slug.as_str())
            })
            .collect();

        if refine_targets.is_empty() {
            eprintln!("No refinement targets (no skills with trigger_context)");
        } else {
            for slug in &refine_targets {
                let contexts = match trigger_map.get(slug.as_str()) {
                    Some(c) => c.clone(),
                    None => continue,
                };

                // Read current description from MD file
                let md_path = drafts_dir.join(format!("{}.md", slug));
                if !md_path.exists() {
                    // Try skills_dir
                    let skill_path = config.skills_dir.join(format!("{}.md", slug));
                    if !skill_path.exists() {
                        eprintln!("  {} -- MD file not found, skipping", slug);
                        continue;
                    }
                }

                let md_path = if drafts_dir.join(format!("{}.md", slug)).exists() {
                    drafts_dir.join(format!("{}.md", slug))
                } else {
                    config.skills_dir.join(format!("{}.md", slug))
                };

                let content = std::fs::read_to_string(&md_path)?;
                let current_desc = util::extract_description_from_md(&content).unwrap_or_default();

                if current_desc.is_empty() {
                    eprintln!("  {} -- description is empty, skipping", slug);
                    continue;
                }

                eprintln!("  {} -- refining... ({} trigger phrases)", slug, contexts.len());

                match refiner::refine_description(&current_desc, &contexts, slug, &config.ai_options)
                {
                    Ok(new_desc) => {
                        if new_desc == current_desc {
                            println!("  {} -- no changes", slug);
                        } else if dry_run {
                            println!("  {} — description diff:", slug);
                            println!("    OLD: {}", current_desc);
                            println!("    NEW: {}", new_desc);
                        } else {
                            // Replace description in MD file
                            let new_content =
                                util::replace_description_in_md(&content, &new_desc);
                            std::fs::write(&md_path, new_content)?;
                            println!("  {} -- description updated", slug);
                            println!("    OLD: {}", current_desc);
                            println!("    NEW: {}", new_desc);
                        }
                    }
                    Err(e) => {
                        eprintln!("  {} -- refinement failed: {}", slug, e);
                    }
                }
            }
        }
    }

    // Summary
    if dry_run {
        eprintln!("\nDry run: no changes written");
        let below = target_slugs
            .iter()
            .filter(|s| score_map.get(s.as_str()).copied().unwrap_or(0.0) < min_score)
            .count();
        if below > 0 {
            eprintln!("{} skills would be rejected (score < {:.2})", below, min_score);
        }
    } else {
        manifest::write_manifest(&drafts_dir, &mf)?;
        eprintln!(
            "\nUpdated {} skills, rejected {} (score < {:.2})",
            updated_count, rejected_count, min_score
        );
        eprintln!("Manifest written to {}", drafts_dir.join("manifest.toml").display());
    }

    Ok(())
}

fn cmd_verify(bundle_path: PathBuf) -> Result<()> {
    let errors = bundle::verify_bundle(&bundle_path)?;

    if errors.is_empty() {
        println!("Bundle integrity: OK");
        let bun = bundle::read_bundle(&bundle_path)?;
        println!(
            "  {} skills, {} patterns",
            bun.skills.len(),
            bun.source.patterns
        );
    } else {
        println!("Bundle integrity: FAILED");
        for err in &errors {
            println!("  {}", err);
        }
    }

    Ok(())
}

fn cmd_validate(bundle_path: PathBuf, public: bool, fix: bool) -> Result<()> {
    if fix {
        let fixed = bundle::fix_bundle(
            &bundle_path,
            &bundle::ValidateOptions {
                public_profile: public,
            },
        )?;
        println!("Auto-fix:");
        println!("  updated files: {}", fixed.updated_files);
        for note in fixed.notes.iter().take(20) {
            println!("  - {}", note);
        }
        if fixed.notes.len() > 20 {
            println!("  - ... {} more", fixed.notes.len() - 20);
        }
        println!();
    }

    let report = bundle::validate_bundle(
        &bundle_path,
        &bundle::ValidateOptions {
            public_profile: public,
        },
    )?;

    println!("Bundle validation:");
    println!("  checked skills: {}", report.checked_skills);
    println!("  errors: {}", report.errors.len());
    println!("  warnings: {}", report.warnings.len());

    if !report.errors.is_empty() {
        println!("\nErrors:");
        for err in &report.errors {
            println!("  - {}", err);
        }
    }
    if !report.warnings.is_empty() {
        println!("\nWarnings:");
        for warn in &report.warnings {
            println!("  - {}", warn);
        }
    }

    if report.errors.is_empty() && report.warnings.is_empty() {
        println!("Result: OK");
    } else if report.errors.is_empty() {
        println!("Result: WARN");
    } else {
        println!("Result: FAILED");
    }

    Ok(())
}

