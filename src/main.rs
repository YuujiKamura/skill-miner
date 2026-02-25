use anyhow::Result;
use clap::{Parser, Subcommand};
use skill_miner::{
    bundle, classifier, compressor, deployer, extractor, generator, history, manifest, parser,
    util, DraftStatus, MineConfig, PipelineStats, PruneOptions,
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

    /// Run full pipeline: scan → classify → extract → generate
    Mine {
        #[arg(short, long, default_value = "30")]
        days: u32,
        #[arg(short, long, default_value = "4")]
        min_messages: usize,
        /// Output directory for generated skills
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Dry run: show what would be generated without writing files
        #[arg(long)]
        dry_run: bool,
        /// Maximum parallel AI calls
        #[arg(long, default_value = "4")]
        parallel: usize,
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
            days,
            min_messages,
            output,
            dry_run,
            parallel,
        } => cmd_mine(&config, days, min_messages, output, dry_run, parallel),
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
            dir,
        } => cmd_export(&config, output, name, author, description, approved_only, dir),
        Command::Import { bundle_path, dir } => cmd_import(&config, bundle_path, dir),
        Command::Verify { bundle_path } => cmd_verify(bundle_path),
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
    let (clusters, _extract_calls) =
        extractor::extract_all_parallel(&groups, None, &config.ai_options, parallel)?;

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
    days: u32,
    min_messages: usize,
    output: Option<PathBuf>,
    dry_run: bool,
    parallel: usize,
) -> Result<()> {
    // Step 1: Parse
    eprintln!("[1/4] Parsing conversations (last {} days)...", days);
    let conversations = parser::parse_all(&config.projects_dir, min_messages, days)?;
    eprintln!("  → {} conversations", conversations.len());

    // Step 2: Compress & Classify
    eprintln!("[2/4] Classifying...");
    let summaries = compressor::compress_all(&conversations);
    let classified = classifier::classify(&summaries, &config.ai_options)?;

    let groups = classifier::group_by_domain(&classified);
    for (domain, convs) in &groups {
        eprintln!("  {} → {} conversations", domain, convs.len());
    }

    // Stats: classify calls = number of batches (50 per batch)
    let classify_calls = (summaries.len() + 49) / 50; // ceil division

    // Build conv_map from already-parsed conversations to avoid re-parsing in extractor
    let conv_map: HashMap<String, &skill_miner::Conversation> = conversations
        .iter()
        .map(|c| (c.id.clone(), c))
        .collect();

    // Step 3: Extract (parallel) — uses conv_map to skip redundant parse
    eprintln!("[3/4] Extracting patterns (parallel)...");
    let (clusters, extract_calls) =
        extractor::extract_all_parallel(&groups, Some(&conv_map), &config.ai_options, parallel)?;
    for cluster in &clusters {
        eprintln!("  {} → {} patterns", cluster.domain, cluster.patterns.len());
    }

    // Step 4: Generate
    eprintln!("[4/4] Generating skills...");
    let mut drafts = generator::generate_skills(&clusters);
    generator::check_existing_skills(&mut drafts, &config.skills_dir)?;

    for draft in &drafts {
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
        let out_dir = output.unwrap_or_else(|| config.skills_dir.join("drafts"));
        std::fs::create_dir_all(&out_dir)?;

        for draft in &drafts {
            let content = generator::format_skill_md(draft);
            let path = out_dir.join(format!("{}.md", draft.name));
            std::fs::write(&path, content)?;
        }

        // Write manifest.toml alongside drafts
        let mf = manifest::create_from_drafts(&drafts, &clusters, &out_dir);
        manifest::write_manifest(&out_dir, &mf)?;

        eprintln!("\nWrote {} drafts + manifest.toml to {}", drafts.len(), out_dir.display());
    } else {
        eprintln!("\nDry run: {} drafts would be generated", drafts.len());
    }

    // Stats summary
    let stats = PipelineStats {
        classify_calls,
        extract_calls,
        total_calls: classify_calls + extract_calls,
    };
    eprintln!();
    eprintln!("=== Stats ===");
    eprintln!(
        "Classify: {} AI calls ({} conversations / 50 per batch)",
        stats.classify_calls,
        summaries.len()
    );
    eprintln!(
        "Extract: {} AI calls ({} domains)",
        stats.extract_calls,
        stats.extract_calls
    );
    eprintln!("Total: {} AI calls", stats.total_calls);

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
        println!(
            "[{:9}] {:<20} {:<12} {} patterns{}",
            e.status.to_string(),
            e.slug,
            e.domain,
            e.pattern_count,
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

fn cmd_export(
    config: &MineConfig,
    output: PathBuf,
    name: String,
    author: Option<String>,
    description: String,
    approved_only: bool,
    dir: Option<PathBuf>,
) -> Result<()> {
    let drafts_dir = resolve_drafts_dir(config, dir);
    let mf = load_or_create_manifest(&drafts_dir)?;

    let opts = bundle::ExportOptions {
        approved_only,
        name,
        author,
        description,
    };

    let bun = bundle::export_bundle(&drafts_dir, &output, &mf, &opts)?;

    println!("=== Exported Bundle ===");
    println!("Name: {}", bun.name);
    println!("Skills: {}", bun.skills.len());
    println!(
        "Source: {} conversations, {} domains, {} patterns",
        bun.source.conversations, bun.source.domains, bun.source.patterns
    );
    println!("Output: {}", output.display());

    Ok(())
}

fn cmd_import(config: &MineConfig, bundle_path: PathBuf, dir: Option<PathBuf>) -> Result<()> {
    let drafts_dir = resolve_drafts_dir(config, dir);
    let mut mf = load_or_create_manifest(&drafts_dir)?;

    let result = bundle::import_bundle(&bundle_path, &drafts_dir, &mut mf)?;

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

    manifest::write_manifest(&drafts_dir, &mf)?;

    eprintln!(
        "\nImport: {} new, {} skipped, {} conflicts",
        result.imported.len(),
        result.skipped.len(),
        result.conflicted.len()
    );

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

