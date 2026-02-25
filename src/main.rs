use anyhow::Result;
use clap::{Parser, Subcommand};
use skill_miner::{classifier, compressor, extractor, generator, history, parser, util, MineConfig, PipelineStats};
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
        eprintln!("\nWrote {} drafts to {}", drafts.len(), out_dir.display());
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

