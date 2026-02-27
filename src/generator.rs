use crate::domains;
use crate::error::SkillMinerError;
use crate::types::{DomainCluster, SkillDraft};
use std::path::Path;

/// Generate skill drafts from domain clusters
pub fn generate_skills(clusters: &[DomainCluster]) -> Vec<SkillDraft> {
    clusters.iter().flat_map(generate_from_cluster).collect()
}

fn generate_from_cluster(cluster: &DomainCluster) -> Vec<SkillDraft> {
    if cluster.patterns.is_empty() {
        return vec![];
    }

    // Use stable slug from domain master instead of ad-hoc slugify
    let name = domains::normalize(&cluster.domain).slug.to_string();
    let description = build_description(cluster);
    let body = build_body(cluster);
    let sources: Vec<String> = cluster
        .patterns
        .iter()
        .flat_map(|p| p.source_ids.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    vec![SkillDraft {
        name,
        description,
        body,
        sources,
        existing_skill: None,
        diff: None,
    }]
}

pub fn build_description(cluster: &DomainCluster) -> String {
    let pattern_summaries: Vec<&str> = cluster
        .patterns
        .iter()
        .take(5)
        .map(|p| p.title.as_str())
        .collect();

    let domain_def = domains::normalize(&cluster.domain);
    let domain_keywords: Vec<&str> = domain_def.keywords.iter().map(|s| s.as_str()).collect();

    format!(
        "{}。({}) {}と言われた時に使用。",
        cluster.domain,
        pattern_summaries.join("、"),
        domain_keywords.join("、")
    )
}

pub fn build_body(cluster: &DomainCluster) -> String {
    let mut body = format!("# {}\n\n", cluster.domain);

    body.push_str(&format!(
        "会話数: {} | パターン数: {}\n\n",
        cluster.conversations.len(),
        cluster.patterns.len()
    ));

    for (i, pattern) in cluster.patterns.iter().enumerate() {
        render_pattern(&mut body, i + 1, pattern, None);
    }

    body
}

/// Render a single pattern entry into a body string.
/// If `score` is provided, it's appended alongside frequency.
fn render_pattern(
    body: &mut String,
    number: usize,
    pattern: &crate::types::KnowledgePattern,
    score: Option<f64>,
) {
    body.push_str(&format!("## {}. {}\n\n", number, pattern.title));
    body.push_str(&format!("{}\n\n", pattern.description));

    if !pattern.steps.is_empty() {
        body.push_str("### 手順\n\n");
        for (j, step) in pattern.steps.iter().enumerate() {
            body.push_str(&format!("{}. {}\n", j + 1, step));
        }
        body.push_str("\n");
    }

    match (pattern.frequency > 1, score) {
        (true, Some(s)) => body.push_str(&format!("出現頻度: {}回 | スコア: {:.2}\n\n", pattern.frequency, s)),
        (true, None) => body.push_str(&format!("出現頻度: {}回\n\n", pattern.frequency)),
        (false, Some(s)) => body.push_str(&format!("スコア: {:.2}\n\n", s)),
        (false, None) => body.push('\n'),
    }
}

/// Rebuild description using scored patterns (sorted by score desc).
/// Filters out patterns with score < 0.05. Falls back to `build_description` if empty.
pub fn rebuild_description_scored(
    cluster: &DomainCluster,
    scored_patterns: &[(usize, f64)],
    max_patterns: usize,
) -> String {
    if scored_patterns.is_empty() {
        return build_description(cluster);
    }

    let pattern_summaries: Vec<&str> = scored_patterns
        .iter()
        .filter(|(_, score)| *score >= 0.05)
        .take(max_patterns)
        .filter_map(|(idx, _)| cluster.patterns.get(*idx).map(|p| p.title.as_str()))
        .collect();

    if pattern_summaries.is_empty() {
        return build_description(cluster);
    }

    let domain_def = domains::normalize(&cluster.domain);
    let domain_keywords: Vec<&str> = domain_def.keywords.iter().map(|s| s.as_str()).collect();

    format!(
        "{}。({}) {}と言われた時に使用。",
        cluster.domain,
        pattern_summaries.join("、"),
        domain_keywords.join("、")
    )
}

/// Rebuild body using scored patterns (sorted by score desc).
/// Includes score display alongside frequency. Filters out patterns with score < 0.05.
/// Falls back to `build_body` if empty.
pub fn rebuild_body_scored(
    cluster: &DomainCluster,
    scored_patterns: &[(usize, f64)],
) -> String {
    if scored_patterns.is_empty() {
        return build_body(cluster);
    }

    let filtered: Vec<(usize, f64)> = scored_patterns
        .iter()
        .filter(|(_, score)| *score >= 0.05)
        .copied()
        .collect();

    if filtered.is_empty() {
        return build_body(cluster);
    }

    let mut body = format!("# {}\n\n", cluster.domain);

    body.push_str(&format!(
        "会話数: {} | パターン数: {}\n\n",
        cluster.conversations.len(),
        filtered.len()
    ));

    for (i, (idx, score)) in filtered.iter().enumerate() {
        if let Some(pattern) = cluster.patterns.get(*idx) {
            render_pattern(&mut body, i + 1, pattern, Some(*score));
        }
    }

    body
}

/// Format a skill draft as a complete .md file.
/// Delegates to `SkillDraft::format_md()`.
pub fn format_skill_md(draft: &SkillDraft) -> String {
    draft.format_md()
}

/// Check existing skills for overlap, read their bodies, and compute diffs.
/// Delegates to `deployer::check_existing_skills()`.
pub fn check_existing_skills(
    drafts: &mut [SkillDraft],
    skills_dir: &Path,
) -> Result<(), SkillMinerError> {
    crate::deployer::check_existing_skills(drafts, skills_dir)
}

/// Parse diff summary from a SkillDraft's diff string.
/// Delegates to `deployer::parse_diff_summary()`.
pub fn parse_diff_summary(diff: &str) -> (usize, usize) {
    crate::deployer::parse_diff_summary(diff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::KnowledgePattern;

    #[test]
    fn test_rebuild_description_scored() {
        let cluster = DomainCluster {
            domain: "テスト".to_string(),
            conversations: vec![],
            patterns: vec![
                KnowledgePattern {
                    title: "High score".to_string(),
                    description: "desc".to_string(),
                    steps: vec![],
                    source_ids: vec![],
                    frequency: 10,
                },
                KnowledgePattern {
                    title: "Low score".to_string(),
                    description: "desc".to_string(),
                    steps: vec![],
                    source_ids: vec![],
                    frequency: 1,
                },
                KnowledgePattern {
                    title: "Zero score".to_string(),
                    description: "desc".to_string(),
                    steps: vec![],
                    source_ids: vec![],
                    frequency: 0,
                },
            ],
        };
        let scored = vec![(0, 0.8), (1, 0.3), (2, 0.01)];
        let desc = rebuild_description_scored(&cluster, &scored, 5);
        assert!(desc.contains("High score"));
        assert!(desc.contains("Low score"));
        assert!(!desc.contains("Zero score")); // filtered by 0.05 threshold
    }

    #[test]
    fn test_rebuild_body_scored() {
        let cluster = DomainCluster {
            domain: "テスト".to_string(),
            conversations: vec![],
            patterns: vec![
                KnowledgePattern {
                    title: "Second".to_string(),
                    description: "desc2".to_string(),
                    steps: vec![],
                    source_ids: vec![],
                    frequency: 2,
                },
                KnowledgePattern {
                    title: "First".to_string(),
                    description: "desc1".to_string(),
                    steps: vec!["step1".to_string()],
                    source_ids: vec![],
                    frequency: 5,
                },
            ],
        };
        // Pattern 1 scored higher, should appear first
        let scored = vec![(1, 0.9), (0, 0.4)];
        let body = rebuild_body_scored(&cluster, &scored);
        let first_pos = body.find("First").unwrap();
        let second_pos = body.find("Second").unwrap();
        assert!(first_pos < second_pos);
        assert!(body.contains("スコア: 0.90"));
    }

    #[test]
    fn test_rebuild_description_scored_empty_fallback() {
        let cluster = DomainCluster {
            domain: "テスト".to_string(),
            conversations: vec![],
            patterns: vec![KnowledgePattern {
                title: "Pattern".to_string(),
                description: "d".to_string(),
                steps: vec![],
                source_ids: vec![],
                frequency: 1,
            }],
        };
        let desc = rebuild_description_scored(&cluster, &[], 5);
        assert!(desc.contains("Pattern")); // fallback to build_description
    }
}
