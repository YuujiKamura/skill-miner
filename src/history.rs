use crate::error::SkillMinerError;
use chrono::Local;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// A lightweight entry from history.jsonl (no full conversation parse needed)
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub display: String,
    pub timestamp: u64,
    pub project: String,
}

/// Parse history.jsonl into lightweight entries (fast: no full conversation parse)
pub fn parse_history(path: &Path) -> Result<Vec<HistoryEntry>, SkillMinerError> {
    let file = std::fs::File::open(path).map_err(|e| {
        SkillMinerError::Parse(format!("opening {}: {}", path.display(), e))
    })?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let display = v
            .get("display")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_string();
        let timestamp = v
            .get("timestamp")
            .and_then(|t| t.as_u64())
            .unwrap_or(0);
        let project = v
            .get("project")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();

        if display.is_empty() {
            continue;
        }

        entries.push(HistoryEntry {
            display,
            timestamp,
            project,
        });
    }

    Ok(entries)
}

/// Filter entries by project path (case-insensitive substring match on Windows paths)
pub fn filter_by_project<'a>(
    entries: &'a [HistoryEntry],
    project: &str,
) -> Vec<&'a HistoryEntry> {
    let project_lower = project.to_lowercase();
    entries
        .iter()
        .filter(|e| e.project.to_lowercase().contains(&project_lower))
        .collect()
}

/// Filter entries within the last N days from now
pub fn filter_by_days(entries: &[HistoryEntry], days_back: u32) -> Vec<&HistoryEntry> {
    if days_back == 0 {
        return entries.iter().collect();
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let cutoff_ms = now_ms.saturating_sub(days_back as u64 * 86_400_000);

    entries
        .iter()
        .filter(|e| e.timestamp >= cutoff_ms)
        .collect()
}

/// Filter entries from today (since midnight local time), sorted by timestamp ascending
pub fn filter_today(entries: &[HistoryEntry]) -> Vec<&HistoryEntry> {
    let today_midnight = Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap();
    let midnight_ms = today_midnight.timestamp_millis() as u64;

    let mut result: Vec<&HistoryEntry> = entries
        .iter()
        .filter(|e| e.timestamp >= midnight_ms)
        .collect();
    result.sort_by_key(|e| e.timestamp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entries() -> Vec<HistoryEntry> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        vec![
            HistoryEntry {
                display: "fix bug".to_string(),
                timestamp: now_ms - 1_000, // 1 second ago
                project: "C:\\Users\\yuuji\\ProjectA".to_string(),
            },
            HistoryEntry {
                display: "add feature".to_string(),
                timestamp: now_ms - 86_400_000 * 10, // 10 days ago
                project: "C:\\Users\\yuuji\\ProjectB".to_string(),
            },
            HistoryEntry {
                display: "old entry".to_string(),
                timestamp: now_ms - 86_400_000 * 60, // 60 days ago
                project: "C:\\Users\\yuuji\\ProjectA".to_string(),
            },
        ]
    }

    #[test]
    fn test_filter_by_project() {
        let entries = sample_entries();
        let filtered = filter_by_project(&entries, "ProjectA");
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].display, "fix bug");
        assert_eq!(filtered[1].display, "old entry");
    }

    #[test]
    fn test_filter_by_project_case_insensitive() {
        let entries = sample_entries();
        let filtered = filter_by_project(&entries, "projecta");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_by_days() {
        let entries = sample_entries();
        let filtered = filter_by_days(&entries, 30);
        assert_eq!(filtered.len(), 2); // 1s ago and 10d ago, not 60d ago
    }

    #[test]
    fn test_filter_by_days_zero_returns_all() {
        let entries = sample_entries();
        let filtered = filter_by_days(&entries, 0);
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn test_filter_today_includes_recent() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let entries = vec![HistoryEntry {
            display: "recent".to_string(),
            timestamp: now_ms - 1_000, // 1 second ago
            project: "test".to_string(),
        }];
        let filtered = filter_today(&entries);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].display, "recent");
    }

    #[test]
    fn test_filter_today_excludes_yesterday() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let entries = vec![HistoryEntry {
            display: "old".to_string(),
            timestamp: now_ms - 86_400_000 * 2, // 2 days ago
            project: "test".to_string(),
        }];
        let filtered = filter_today(&entries);
        assert_eq!(filtered.len(), 0);
    }

    #[test]
    fn test_filter_today_sorted_ascending() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let entries = vec![
            HistoryEntry {
                display: "a".to_string(),
                timestamp: now_ms - 3_000,
                project: "test".to_string(),
            },
            HistoryEntry {
                display: "b".to_string(),
                timestamp: now_ms - 1_000,
                project: "test".to_string(),
            },
            HistoryEntry {
                display: "c".to_string(),
                timestamp: now_ms - 2_000,
                project: "test".to_string(),
            },
        ];
        let filtered = filter_today(&entries);
        assert_eq!(filtered.len(), 3);
        assert!(filtered[0].timestamp <= filtered[1].timestamp);
        assert!(filtered[1].timestamp <= filtered[2].timestamp);
        assert_eq!(filtered[0].display, "a"); // oldest first
        assert_eq!(filtered[1].display, "c");
        assert_eq!(filtered[2].display, "b"); // newest last
    }
}
