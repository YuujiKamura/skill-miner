use crate::{compressor, history, parser, util, Role};
use cli_ai_analyzer::prompt;
use std::collections::HashMap;

#[derive(serde::Deserialize, Clone)]
pub struct AiSlotSummary {
    pub slot: String,
    pub target: String,
    pub summary: String,
}

/// Context for a single conversation within a slot
pub struct ConvContext {
    pub cwd: Option<String>,
    pub first_message: String,
    pub first_response: String,
    pub files_touched: Vec<String>,
}

/// Aggregated context for a time slot
pub struct SlotContext {
    pub date: String,
    pub slot: String,
    pub conversations: Vec<ConvContext>,
}

pub fn build_slot_contexts(
    config: &crate::MineConfig,
    days: u32,
    slot_minutes: i32,
) -> Vec<SlotContext> {
    use chrono::Timelike;

    let conversations = match parser::parse_all(&config.projects_dir, 1, days) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Warning: could not parse conversations: {}", e);
            return Vec::new();
        }
    };

    let mut slot_map: HashMap<(String, String), Vec<ConvContext>> = HashMap::new();

    for conv in &conversations {
        let Some(ts) = conv.start_time else { continue };
        let dt = ts.with_timezone(&chrono::Local);
        let date = dt.format("%Y-%m-%d").to_string();
        let minute = dt.hour() as i32 * 60 + dt.minute() as i32;
        let slot_start = (minute / slot_minutes) * slot_minutes;
        let slot = format!("{:02}:{:02}", slot_start / 60, slot_start % 60);

        let first_message = conv
            .messages
            .iter()
            .find(|m| m.role == Role::User && !m.content.trim().is_empty())
            .map(|m| util::truncate(&m.content, 500))
            .unwrap_or_default();

        let first_response = conv
            .messages
            .iter()
            .find(|m| m.role == Role::Assistant && !m.content.trim().is_empty())
            .map(|m| util::truncate(&m.content, 500))
            .unwrap_or_default();

        let summary = compressor::compress(conv);
        let files_touched: Vec<String> = summary
            .files_touched
            .into_iter()
            .filter(|f| !is_noise_target(f))
            .take(5)
            .collect();

        slot_map.entry((date, slot)).or_default().push(ConvContext {
            cwd: conv.cwd.clone(),
            first_message,
            first_response,
            files_touched,
        });
    }

    let mut slots: Vec<SlotContext> = slot_map
        .into_iter()
        .map(|((date, slot), conversations)| SlotContext {
            date,
            slot,
            conversations,
        })
        .collect();
    slots.sort_by(|a, b| (&a.date, &a.slot).cmp(&(&b.date, &b.slot)));
    slots
}

pub fn summarize_slots_with_ai(
    slot_contexts: &[SlotContext],
    ai_options: &cli_ai_analyzer::AnalyzeOptions,
) -> HashMap<(String, String), AiSlotSummary> {
    if slot_contexts.is_empty() {
        return HashMap::new();
    }

    // Group by date for per-day AI calls
    let mut by_date: std::collections::BTreeMap<String, Vec<&SlotContext>> =
        std::collections::BTreeMap::new();
    for sc in slot_contexts {
        by_date.entry(sc.date.clone()).or_default().push(sc);
    }

    let mut result: HashMap<(String, String), AiSlotSummary> = HashMap::new();

    for (date, day_slots) in &by_date {
        let mut context_text = String::new();
        for sc in day_slots {
            context_text.push_str(&format!("Slot {}:\n", sc.slot));
            for (i, cc) in sc.conversations.iter().enumerate() {
                context_text.push_str(&format!("  Conv {}:\n", i + 1));
                if let Some(ref cwd) = cc.cwd {
                    context_text.push_str(&format!("    cwd: {}\n", cwd));
                }
                if !cc.first_message.is_empty() {
                    context_text.push_str(&format!("    User: {}\n", cc.first_message));
                }
                if !cc.first_response.is_empty() {
                    context_text.push_str(&format!("    AI: {}\n", cc.first_response));
                }
                if !cc.files_touched.is_empty() {
                    let files: Vec<_> = cc.files_touched.iter().map(|f| f.as_str()).collect();
                    context_text.push_str(&format!("    files: [{}]\n", files.join(", ")));
                }
            }
            context_text.push('\n');
        }

        let prompt_text = format!(
            "You are analyzing work logs for {date}.\n\
            Infer what project/file was being worked on from cwd, user message, AI response, and files_touched.\n\
            Return ONLY a JSON array. Each item must be:\n\
            {{\"slot\":\"HH:MM\",\"target\":\"...\",\"summary\":\"...\"}}\n\
            Rules:\n\
            - target: The project or file being worked on. Use cwd directory name (e.g. skill-miner) or specific file paths from files_touched.\n\
            - NEVER use environment/tool paths (node_modules, .claude/plugins/cache, AppData) as target.\n\
            - summary: Explain in detail what was done: goal, changes, decisions, and results.\n\
            - Output Japanese.\n\
            \n\
            Context by slot:\n{context_text}"
        );

        if let Ok(response) = prompt(&prompt_text, ai_options.clone()) {
            if let Ok(parsed) = util::parse_json_response::<AiSlotSummary>(&response) {
                for item in parsed {
                    let slot_key = normalize_slot(&item.slot);
                    result.insert((date.clone(), slot_key), item);
                }
            }
        }
    }

    result
}

pub fn print_summary_timeline(
    entries: &[&history::HistoryEntry],
    slot_minutes: i32,
    ai_summaries: &HashMap<(String, String), AiSlotSummary>,
) {
    use chrono::Timelike;
    use std::collections::BTreeMap;

    let mut grouped_by_day: BTreeMap<String, BTreeMap<String, Vec<&history::HistoryEntry>>> =
        BTreeMap::new();
    for entry in entries {
        let dt = chrono::DateTime::from_timestamp_millis(entry.timestamp as i64)
            .unwrap_or_default()
            .with_timezone(&chrono::Local);

        let date = dt.format("%Y-%m-%d").to_string();
        let minute = dt.hour() as i32 * 60 + dt.minute() as i32;
        let slot_start = (minute / slot_minutes) * slot_minutes;
        let slot_label = format!("{:02}:{:02}", slot_start / 60, slot_start % 60);

        grouped_by_day
            .entry(date)
            .or_default()
            .entry(slot_label)
            .or_default()
            .push(*entry);
    }

    let mut first_day = true;
    for (date, slots) in grouped_by_day {
        if !first_day {
            println!();
        }
        first_day = false;
        println!("## {}", date);

        for (slot, items) in slots {
            println!("- {} ({} sessions)", slot, items.len());

            if let Some(ai) = ai_summaries.get(&(date.clone(), slot.clone())) {
                let target = if is_noise_target(&ai.target) {
                    "対象不明".to_string()
                } else {
                    ai.target.clone()
                };
                println!("  - 対象: {}", util::truncate(&target, 180));
                println!("  - 内容: {}", util::truncate(&ai.summary, 180));
            } else {
                println!("  - 対象: (AI要約なし)");
            }

            for quote in extract_display_quotes(&items).iter().take(2) {
                println!("  - 抜粋: {}", util::truncate(quote, 160));
            }
        }
    }
}

/// Normalize slot string to "HH:MM" format (zero-padded)
pub fn normalize_slot(slot: &str) -> String {
    let parts: Vec<&str> = slot.split(':').collect();
    if parts.len() >= 2 {
        if let (Ok(h), Ok(m)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
            return format!("{:02}:{:02}", h, m);
        }
    }
    slot.to_string()
}

pub fn is_noise_target(s: &str) -> bool {
    let l = s.to_lowercase();
    l.contains("node_modules")
        || l.contains("appdata")
        || l.contains(".claude/plugins/cache")
        || l.contains(".claude\\plugins\\cache")
        || l.contains("対象不明（会話テキストのみでは特定不可）")
}

pub fn extract_display_quotes(items: &[&history::HistoryEntry]) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();
    for item in items {
        let s = item.display.replace('\n', " ").replace('\r', " ");
        let s = s.trim().to_string();
        if s.is_empty() || s.len() < 15 {
            continue;
        }
        // Skip slash commands
        if s.starts_with('/') {
            continue;
        }
        // Skip generic short directives
        if is_generic_directive(&s) {
            continue;
        }
        if !candidates.iter().any(|q| q == &s) {
            candidates.push(s);
        }
    }
    // Prefer longer, more substantive quotes
    candidates.sort_by(|a, b| b.len().cmp(&a.len()));
    candidates.into_iter().take(2).collect()
}

fn is_generic_directive(s: &str) -> bool {
    let generic = [
        "チーム作ってやれ", "つづきどうぞ", "どうぞ", "続けて", "やれ", "頼む",
        "プランモード", "続き", "はい", "OK", "ok", "おk",
    ];
    generic.iter().any(|g| s.trim() == *g)
}
