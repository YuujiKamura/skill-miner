//! Tool coverage checker: detect user projects referenced in conversations
//! and report which ones lack corresponding skills.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A user project detected from file paths in conversations.
#[derive(Debug, Clone)]
pub struct DetectedProject {
    pub name: String,
    pub path: PathBuf,
    /// Number of conversations that touched files in this project
    pub conversation_count: usize,
}

/// Check which user projects were referenced in conversations but lack skills.
///
/// Returns a list of projects that have no corresponding skill file.
pub fn find_uncovered_projects(
    files_touched: &[Vec<String>],
    skills_dir: &Path,
    home_dir: &Path,
) -> Vec<DetectedProject> {
    // 1. Extract project names from file paths
    let mut project_counts: HashMap<String, usize> = HashMap::new();
    let mut project_paths: HashMap<String, PathBuf> = HashMap::new();

    let home_str = home_dir.to_string_lossy().replace('\\', "/");
    let home_variants = vec![
        home_str.clone(),
        format!("/c{}", &home_str[2..]), // C:\Users\x -> /c/Users/x
        home_str.replace("C:", "/c"),
    ];

    for conv_files in files_touched {
        let mut seen_in_conv: HashSet<String> = HashSet::new();

        for file in conv_files {
            let normalized = file.replace('\\', "/");

            // Find which home variant matches
            let relative = home_variants
                .iter()
                .find_map(|h| normalized.strip_prefix(&format!("{}/", h)))
                .or_else(|| normalized.strip_prefix("~/"));

            if let Some(rel) = relative {
                // First path component = project directory name
                if let Some(project_name) = rel.split('/').next() {
                    // Skip hidden dirs, temp dirs, etc.
                    if project_name.starts_with('.')
                        || project_name == "AppData"
                        || project_name == "Desktop"
                        || project_name == "Documents"
                        || project_name == "Downloads"
                    {
                        continue;
                    }

                    if seen_in_conv.insert(project_name.to_string()) {
                        *project_counts.entry(project_name.to_string()).or_insert(0) += 1;
                        project_paths
                            .entry(project_name.to_string())
                            .or_insert_with(|| home_dir.join(project_name));
                    }
                }
            }
        }
    }

    // 2. Check which projects have skills
    let existing_skills = load_skill_names(skills_dir);

    // 3. Filter to uncovered projects (no matching skill by name or substring)
    let mut uncovered: Vec<DetectedProject> = project_counts
        .into_iter()
        .filter(|(name, _)| !has_matching_skill(name, &existing_skills))
        .map(|(name, count)| DetectedProject {
            path: project_paths.remove(&name).unwrap_or_default(),
            name,
            conversation_count: count,
        })
        .collect();

    // Sort by conversation count (most referenced first)
    uncovered.sort_by(|a, b| b.conversation_count.cmp(&a.conversation_count));
    uncovered
}

/// Load all skill names from the skills directory.
fn load_skill_names(skills_dir: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Ok(entries) = std::fs::read_dir(skills_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Some(stem) = path.file_stem() {
                    names.insert(stem.to_string_lossy().to_lowercase());
                }
            }
        }
    }
    names
}

/// Check if a project name matches any existing skill (case-insensitive, substring).
fn has_matching_skill(project_name: &str, skills: &HashSet<String>) -> bool {
    let lower = project_name.to_lowercase();
    // Exact match
    if skills.contains(&lower) {
        return true;
    }
    // Substring match (either direction)
    for skill in skills {
        if skill.contains(&lower) || lower.contains(skill.as_str()) {
            return true;
        }
    }
    false
}

/// Format uncovered projects as a report string.
pub fn format_report(uncovered: &[DetectedProject]) -> String {
    if uncovered.is_empty() {
        return String::new();
    }

    let mut report = String::from("\n=== Tool Coverage ===\n");
    report.push_str("以下のプロジェクトはスキル未作成:\n");
    for proj in uncovered {
        report.push_str(&format!(
            "  {} ({}会話で使用) → {}\n",
            proj.name,
            proj.conversation_count,
            proj.path.display()
        ));
    }
    report.push_str("\"ツール棚卸し\" でスキル化を検討\n");
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_extract_project_from_paths() {
        let home = PathBuf::from("C:/Users/testuser");
        let skills_dir = TempDir::new().unwrap();

        let files = vec![vec![
            "C:/Users/testuser/my-tool/src/main.rs".to_string(),
            "C:/Users/testuser/my-tool/Cargo.toml".to_string(),
            "C:/Users/testuser/other-project/index.js".to_string(),
        ]];

        let uncovered = find_uncovered_projects(&files, skills_dir.path(), &home);
        let names: Vec<&str> = uncovered.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"my-tool"));
        assert!(names.contains(&"other-project"));
    }

    #[test]
    fn test_covered_project_excluded() {
        let home = PathBuf::from("C:/Users/testuser");
        let skills_dir = TempDir::new().unwrap();

        // Create a skill file for my-tool
        fs::write(skills_dir.path().join("my-tool.md"), "# My Tool").unwrap();

        let files = vec![vec![
            "C:/Users/testuser/my-tool/src/main.rs".to_string(),
        ]];

        let uncovered = find_uncovered_projects(&files, skills_dir.path(), &home);
        assert!(uncovered.is_empty());
    }

    #[test]
    fn test_hidden_dirs_skipped() {
        let home = PathBuf::from("C:/Users/testuser");
        let skills_dir = TempDir::new().unwrap();

        let files = vec![vec![
            "C:/Users/testuser/.claude/settings.json".to_string(),
            "C:/Users/testuser/AppData/Local/foo.txt".to_string(),
        ]];

        let uncovered = find_uncovered_projects(&files, skills_dir.path(), &home);
        assert!(uncovered.is_empty());
    }
}
