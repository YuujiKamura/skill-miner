use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::types::{DepType, DependencyGraph, GraphNode, RawRef, SkillDependency};

/// Extract references from markdown content (pure function, no I/O).
///
/// Detects three kinds of references:
/// - MarkdownLink: `[text](target.md)` or `[text](target)` (no extension)
/// - SkillRef: backtick-quoted identifiers near skill keywords (skill, Skill)
/// - ProjectPath: `~/project-name/` or Windows drive paths like `C:\Users\...\project\`
pub fn extract_refs(content: &str) -> Vec<RawRef> {
    let mut refs = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        let line_num = line_idx + 1;

        // ── MarkdownLink ──
        extract_markdown_links(line, line_num, &mut refs);

        // ── SkillRef ──
        extract_skill_refs(line, line_num, &mut refs);

        // ── ProjectPath ──
        extract_project_paths(line, line_num, &mut refs);
    }

    refs
}

/// Extract `[text](target)` markdown links where target looks like a local file
/// (ends with .md, or has no extension and no protocol).
fn extract_markdown_links(line: &str, line_num: usize, refs: &mut Vec<RawRef>) {
    let mut search_from = 0;

    while search_from < line.len() {
        // Find '['
        let bracket_open = match line[search_from..].find('[') {
            Some(pos) => search_from + pos,
            None => break,
        };

        // Find '](' after it
        let bracket_close = match line[bracket_open..].find("](") {
            Some(pos) => bracket_open + pos,
            None => {
                search_from = bracket_open + 1;
                continue;
            }
        };

        let paren_open = bracket_close + 2; // position right after ']('

        // Find closing ')'
        let paren_close = match line[paren_open..].find(')') {
            Some(pos) => paren_open + pos,
            None => {
                search_from = paren_open;
                continue;
            }
        };

        let target = &line[paren_open..paren_close];
        search_from = paren_close + 1;

        // Skip URLs (http://, https://, mailto:, etc.)
        if target.contains("://") || target.starts_with("mailto:") {
            continue;
        }

        // Skip empty targets and anchors-only
        if target.is_empty() || target.starts_with('#') {
            continue;
        }

        // Accept: ends with .md, or has no extension (no '.' in the filename part)
        let filename = target.rsplit('/').next().unwrap_or(target);
        let filename = filename.rsplit('\\').next().unwrap_or(filename);
        let is_md = filename.ends_with(".md");
        let has_no_ext = !filename.contains('.');

        if is_md || has_no_ext {
            refs.push(RawRef {
                target: target.to_string(),
                ref_type: DepType::MarkdownLink,
                line: line_num,
            });
        }
    }
}

/// Extract skill references: backtick-quoted identifiers near skill keywords.
/// Patterns:
///   - skill `skill-name`
///   - see skill skill-name
///   - Skill `name`
fn extract_skill_refs(line: &str, line_num: usize, refs: &mut Vec<RawRef>) {
    let lower = line.to_lowercase();

    // Find all occurrences of skill-related keywords
    let keywords = ["skill"];

    for keyword in &keywords {
        let search_line = if *keyword == "skill" { &lower } else { line };
        let mut pos = 0;

        while let Some(found) = search_line[pos..].find(keyword) {
            let kw_start = pos + found;
            let kw_end = kw_start + keyword.len();
            pos = kw_end;

            // Look for a backtick-quoted identifier after the keyword
            // Allow some whitespace/punctuation between keyword and backtick
            let after = &line[kw_end..];
            let trimmed = after.trim_start();

            if let Some(rest) = trimmed.strip_prefix('`') {
                if let Some(end_tick) = rest.find('`') {
                    let skill_name = &rest[..end_tick];
                    if !skill_name.is_empty() && is_valid_skill_name(skill_name) {
                        refs.push(RawRef {
                            target: skill_name.to_string(),
                            ref_type: DepType::SkillRef,
                            line: line_num,
                        });
                    }
                }
            } else {
                // Also try bare word after keyword (e.g., "skill skill-name reference")
                // Take the next whitespace-delimited token, but require a hyphen
                // to avoid false positives like "skill level" or "skill-miner is a tool"
                let next_token = trimmed.split_whitespace().next().unwrap_or("");
                if !next_token.is_empty()
                    && next_token.contains('-')
                    && is_valid_skill_name(next_token)
                    && !next_token.starts_with('[')
                    && !next_token.starts_with('(')
                    && !next_token.starts_with('`')
                {
                    refs.push(RawRef {
                        target: next_token.to_string(),
                        ref_type: DepType::SkillRef,
                        line: line_num,
                    });
                }
            }
        }
    }
}

/// Check if a string looks like a valid skill name (slug-like identifier).
fn is_valid_skill_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Extract project path references:
/// - `~/project-name/`
/// - Windows drive paths containing `\Users\` (e.g. `C:\Users\foo\project\`)
fn extract_project_paths(line: &str, line_num: usize, refs: &mut Vec<RawRef>) {
    // ~/project-name/ pattern
    let mut pos = 0;
    while let Some(found) = line[pos..].find("~/") {
        let start = pos + found;
        let after = &line[start + 2..];

        // Capture path until whitespace or end
        let end = after
            .find(|c: char| c.is_whitespace() || c == ')' || c == ']' || c == '`' || c == '"')
            .unwrap_or(after.len());

        let path = &line[start..start + 2 + end];
        if path.len() > 2 {
            refs.push(RawRef {
                target: path.to_string(),
                ref_type: DepType::ProjectPath,
                line: line_num,
            });
        }

        pos = start + 2 + end;
    }

    // Windows drive path: letter:\Users\ ...
    pos = 0;
    while pos + 3 < line.len() {
        // Look for X:\Users\ or X:/Users/
        let remaining = &line[pos..];
        let found = remaining
            .find(":\\Users\\")
            .or_else(|| remaining.find(":/Users/"));

        match found {
            Some(colon_pos) => {
                let abs_colon = pos + colon_pos;
                // The drive letter is one char before ':'
                if abs_colon == 0 || !line.as_bytes()[abs_colon - 1].is_ascii_alphabetic() {
                    pos = abs_colon + 1;
                    continue;
                }
                let start = abs_colon - 1;

                // Capture until whitespace or certain delimiters
                let after_start = &line[start..];
                let end = after_start
                    .find(|c: char| c.is_whitespace() || c == ')' || c == ']' || c == '`')
                    .unwrap_or(after_start.len());

                let path = &line[start..start + end];
                if path.len() > 3 {
                    refs.push(RawRef {
                        target: path.to_string(),
                        ref_type: DepType::ProjectPath,
                        line: line_num,
                    });
                }

                pos = start + end;
            }
            None => break,
        }
    }
}

/// Resolve a RawRef's target to a normalized relative path for display.
///
/// When `skills_dir` is provided, SkillRef targets are resolved to
/// `skills_dir/{name}.md`; otherwise the bare name is returned.
pub fn resolve_ref(base_dir: &Path, skills_dir: Option<&Path>, raw: &RawRef) -> String {
    match raw.ref_type {
        DepType::MarkdownLink => {
            let resolved = base_dir.join(&raw.target);
            crate::util::normalize_path(&resolved)
        }
        DepType::SkillRef => match skills_dir {
            Some(sd) => {
                let target_path = sd.join(format!("{}.md", raw.target));
                crate::util::normalize_path(&target_path)
            }
            None => raw.target.clone(),
        },
        DepType::ProjectPath => raw.target.clone(),
    }
}

/// Build the full dependency graph from skill/memory/CLAUDE.md files.
/// Thin I/O wrapper: collects files, reads contents, delegates to `build_graph_from_contents`.
pub fn build_graph(
    skills_dir: &Path,
    memory_dirs: &[PathBuf],
    claude_md_paths: &[PathBuf],
) -> DependencyGraph {
    // Step 1: Collect all files
    let mut all_files: Vec<PathBuf> = Vec::new();

    // skills_dir/**/*.md
    if skills_dir.is_dir() {
        collect_md_files_recursive(skills_dir, &mut all_files);
    }

    // memory_dirs/*.md (non-recursive)
    for mdir in memory_dirs {
        if mdir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(mdir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().map_or(false, |e| e == "md") && p.is_file() {
                        all_files.push(p);
                    }
                }
            }
        }
    }

    // claude_md paths
    for p in claude_md_paths {
        if p.is_file() {
            all_files.push(p.clone());
        }
    }

    // Deduplicate by canonical path (best-effort)
    let mut seen = HashSet::new();
    all_files.retain(|p| {
        let key = p.canonicalize().unwrap_or_else(|_| p.clone());
        seen.insert(key)
    });

    // Step 2: Read all files into memory
    let mut contents: HashMap<PathBuf, String> = HashMap::new();
    for file_path in &all_files {
        if let Ok(content) = std::fs::read_to_string(file_path) {
            contents.insert(file_path.clone(), content);
        }
    }

    // Step 3: Delegate to pure function
    build_graph_from_contents(&contents, skills_dir)
}

/// Build dependency graph from pre-loaded file contents (pure function, no I/O).
pub fn build_graph_from_contents(
    contents: &HashMap<PathBuf, String>,
    skills_dir: &Path,
) -> DependencyGraph {
    let file_keys: Vec<(PathBuf, String)> = contents
        .keys()
        .map(|p| (p.clone(), crate::util::normalize_path(p)))
        .collect();

    // Build a set of known file paths for broken-link detection
    let known_files: HashSet<String> = file_keys.iter().map(|(_, k)| k.clone()).collect();

    let mut all_deps: Vec<SkillDependency> = Vec::new();

    for (file_path, file_key) in &file_keys {
        let content = match contents.get(file_path) {
            Some(c) => c,
            None => continue,
        };

        let base_dir = file_path.parent().unwrap_or(Path::new("."));
        let raw_refs = extract_refs(content);

        for raw in &raw_refs {
            let resolved = resolve_ref(base_dir, Some(skills_dir), &raw);

            all_deps.push(SkillDependency {
                from: file_key.clone(),
                to: resolved,
                dep_type: raw.ref_type.clone(),
                line: raw.line,
            });
        }
    }

    // Build nodes with outgoing/incoming
    let mut outgoing_map: HashMap<String, Vec<SkillDependency>> = HashMap::new();
    let mut incoming_map: HashMap<String, Vec<SkillDependency>> = HashMap::new();
    let mut broken_links: Vec<SkillDependency> = Vec::new();

    for dep in &all_deps {
        outgoing_map
            .entry(dep.from.clone())
            .or_default()
            .push(dep.clone());

        // Check if the target is a known file (skip ProjectPath — those are external)
        if dep.dep_type != DepType::ProjectPath {
            if known_files.contains(&dep.to) {
                incoming_map
                    .entry(dep.to.clone())
                    .or_default()
                    .push(dep.clone());
            } else {
                broken_links.push(dep.clone());
            }
        }
    }

    // Build GraphNode for each file
    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut has_any_ref: HashSet<String> = HashSet::new();

    for (_, file_key) in &file_keys {
        let outgoing = outgoing_map.remove(file_key).unwrap_or_default();
        let incoming = incoming_map.remove(file_key).unwrap_or_default();

        if !outgoing.is_empty() || !incoming.is_empty() {
            has_any_ref.insert(file_key.clone());
        }

        for dep in &outgoing {
            has_any_ref.insert(dep.from.clone());
        }

        nodes.push(GraphNode {
            path: file_key.clone(),
            outgoing,
            incoming,
        });
    }

    // Detect orphans (files with no incoming AND no outgoing refs)
    let orphans: Vec<String> = file_keys
        .iter()
        .map(|(_, k)| k)
        .filter(|k| !has_any_ref.contains(*k))
        .cloned()
        .collect();

    DependencyGraph {
        nodes,
        broken_links,
        orphans,
    }
}

/// Recursively collect .md files from a directory.
fn collect_md_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_md_files_recursive(&p, out);
        } else if p.extension().map_or(false, |e| e == "md") && p.is_file() {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_extract_markdown_link() {
        let content = "See [details](path.md) for more info.\n";
        let refs = extract_refs(content);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target, "path.md");
        assert_eq!(refs[0].ref_type, DepType::MarkdownLink);
        assert_eq!(refs[0].line, 1);
    }

    #[test]
    fn test_extract_markdown_link_no_ext() {
        let content = "Link to [name](subdir/file) here.\n";
        let refs = extract_refs(content);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target, "subdir/file");
        assert_eq!(refs[0].ref_type, DepType::MarkdownLink);
    }

    #[test]
    fn test_extract_markdown_link_skip_url() {
        let content = "Visit [site](https://example.com) and [doc](notes.md).\n";
        let refs = extract_refs(content);
        // Only notes.md, not the URL
        let md_links: Vec<_> = refs
            .iter()
            .filter(|r| r.ref_type == DepType::MarkdownLink)
            .collect();
        assert_eq!(md_links.len(), 1);
        assert_eq!(md_links[0].target, "notes.md");
    }

    #[test]
    fn test_extract_skill_ref() {
        let content = "See skill `contactsheet-pairing` for details\n";
        let refs = extract_refs(content);
        let skill_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.ref_type == DepType::SkillRef)
            .collect();
        assert_eq!(skill_refs.len(), 1);
        assert_eq!(skill_refs[0].target, "contactsheet-pairing");
        assert_eq!(skill_refs[0].line, 1);
    }

    #[test]
    fn test_extract_skill_ref_english() {
        let content = "Use Skill `my-tool` for this.\n";
        let refs = extract_refs(content);
        let skill_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.ref_type == DepType::SkillRef)
            .collect();
        assert_eq!(skill_refs.len(), 1);
        assert_eq!(skill_refs[0].target, "my-tool");
    }

    #[test]
    fn test_extract_project_path_unix() {
        let content = "Project is at ~/my-project/ for development.\n";
        let refs = extract_refs(content);
        let proj_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.ref_type == DepType::ProjectPath)
            .collect();
        assert_eq!(proj_refs.len(), 1);
        assert_eq!(proj_refs[0].target, "~/my-project/");
    }

    #[test]
    fn test_extract_project_path_windows() {
        let content = "Located at C:\\Users\\yuuji\\skill-miner\\ here.\n";
        let refs = extract_refs(content);
        let proj_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.ref_type == DepType::ProjectPath)
            .collect();
        assert_eq!(proj_refs.len(), 1);
        assert!(proj_refs[0].target.contains("Users"));
    }

    #[test]
    fn test_extract_mixed() {
        let content = "\
# My Skill

See [details](accuracy-findings.md) for accuracy data.
See skill `box-overlay` for details
Project at ~/tonsuu-checker/ for reference.
";
        let refs = extract_refs(content);

        let md_links: Vec<_> = refs
            .iter()
            .filter(|r| r.ref_type == DepType::MarkdownLink)
            .collect();
        let skill_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.ref_type == DepType::SkillRef)
            .collect();
        let proj_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.ref_type == DepType::ProjectPath)
            .collect();

        assert_eq!(md_links.len(), 1);
        assert_eq!(md_links[0].target, "accuracy-findings.md");

        assert_eq!(skill_refs.len(), 1);
        assert_eq!(skill_refs[0].target, "box-overlay");

        assert_eq!(proj_refs.len(), 1);
        assert_eq!(proj_refs[0].target, "~/tonsuu-checker/");
    }

    #[test]
    fn test_extract_no_refs() {
        let content = "This is plain text with no references at all.\nJust normal content.\n";
        let refs = extract_refs(content);
        assert!(refs.is_empty());
    }

    #[test]
    fn test_build_graph_simple() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let memory_dir = tmp.path().join("memory");
        fs::create_dir_all(&memory_dir).unwrap();

        // Create skill file that references another skill
        fs::write(
            skills_dir.join("skill-a.md"),
            "# Skill A\nSee [details](../memory/notes.md) for more.\n",
        )
        .unwrap();

        // Create memory file that references a skill
        fs::write(
            memory_dir.join("notes.md"),
            "# Notes\nSee skill `skill-a` for details\n",
        )
        .unwrap();

        // Create an orphan file
        fs::write(
            memory_dir.join("orphan.md"),
            "# Orphan\nNo references here.\n",
        )
        .unwrap();

        let graph = build_graph(
            &skills_dir,
            &[memory_dir.clone()],
            &[],
        );

        // Should have 3 nodes
        assert_eq!(graph.nodes.len(), 3);

        // skill-a.md -> notes.md (MarkdownLink)
        let skill_a_node = graph
            .nodes
            .iter()
            .find(|n| n.path.contains("skill-a.md"))
            .expect("skill-a.md node should exist");
        assert!(
            !skill_a_node.outgoing.is_empty(),
            "skill-a should have outgoing refs"
        );
        assert!(
            skill_a_node
                .outgoing
                .iter()
                .any(|d| d.to.contains("notes.md")),
            "skill-a should reference notes.md"
        );

        // notes.md -> skill-a.md (SkillRef)
        let notes_node = graph
            .nodes
            .iter()
            .find(|n| n.path.contains("notes.md") && !n.path.contains("orphan"))
            .expect("notes.md node should exist");
        assert!(
            !notes_node.outgoing.is_empty(),
            "notes.md should have outgoing refs"
        );
        assert!(
            notes_node
                .outgoing
                .iter()
                .any(|d| d.to.contains("skill-a.md")),
            "notes.md should reference skill-a"
        );

        // orphan.md should be in orphans
        assert!(
            graph.orphans.iter().any(|o| o.contains("orphan.md")),
            "orphan.md should be detected as orphan"
        );

        // No broken links in this setup (cross-refs should resolve)
        // broken_links are for targets that don't exist in the known file set
        // notes.md is referenced from skill-a.md with a relative path, which resolves
    }

    #[test]
    fn test_extract_multiple_links_same_line() {
        let content = "See [a](one.md) and [b](two.md) together.\n";
        let refs = extract_refs(content);
        let md_links: Vec<_> = refs
            .iter()
            .filter(|r| r.ref_type == DepType::MarkdownLink)
            .collect();
        assert_eq!(md_links.len(), 2);
        assert_eq!(md_links[0].target, "one.md");
        assert_eq!(md_links[1].target, "two.md");
    }

    #[test]
    fn test_resolve_ref_markdown() {
        let base = Path::new("/home/user/memory");
        let raw = RawRef {
            target: "details.md".to_string(),
            ref_type: DepType::MarkdownLink,
            line: 1,
        };
        let resolved = resolve_ref(base, None, &raw);
        assert!(resolved.contains("memory"));
        assert!(resolved.ends_with("details.md"));
    }

    #[test]
    fn test_resolve_ref_skill() {
        let base = Path::new("/home/user/memory");
        let skills = Path::new("/home/user/skills");
        let raw = RawRef {
            target: "box-overlay".to_string(),
            ref_type: DepType::SkillRef,
            line: 5,
        };

        // With skills_dir: resolves to skills_dir/{name}.md
        let resolved = resolve_ref(base, Some(skills), &raw);
        assert!(
            resolved.ends_with("skills/box-overlay.md"),
            "expected path ending with skills/box-overlay.md, got: {resolved}"
        );

        // Without skills_dir: returns bare name
        let resolved_bare = resolve_ref(base, None, &raw);
        assert_eq!(resolved_bare, "box-overlay");
    }

    #[test]
    fn test_build_graph_from_contents_pure() {
        // Pure function test — no filesystem needed
        let skills_dir = Path::new("/test/skills");

        let mut contents = HashMap::new();
        contents.insert(
            PathBuf::from("/test/skills/skill-a.md"),
            "# Skill A\nSee [notes](../memory/notes.md) for details.\n".to_string(),
        );
        contents.insert(
            PathBuf::from("/test/memory/notes.md"),
            "# Notes\nSee skill `skill-a` for details\n".to_string(),
        );
        contents.insert(
            PathBuf::from("/test/memory/orphan.md"),
            "# Orphan\nNo references here.\n".to_string(),
        );

        let graph = build_graph_from_contents(&contents, skills_dir);

        // Should have 3 nodes
        assert_eq!(graph.nodes.len(), 3);

        // skill-a -> notes.md (MarkdownLink)
        let skill_a = graph.nodes.iter().find(|n| n.path.contains("skill-a")).unwrap();
        assert!(!skill_a.outgoing.is_empty());
        assert!(skill_a.outgoing.iter().any(|d| d.to.contains("notes.md")));

        // notes.md -> skill-a (SkillRef)
        let notes = graph.nodes.iter().find(|n| n.path.contains("notes.md") && !n.path.contains("orphan")).unwrap();
        assert!(!notes.outgoing.is_empty());
        assert!(notes.outgoing.iter().any(|d| d.to.contains("skill-a")));

        // orphan detected
        assert!(graph.orphans.iter().any(|o| o.contains("orphan")));
    }
}
