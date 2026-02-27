// Scorer module: compute consolidation scores for skills and patterns.
//
// Scoring formula for skills:
//   fire_score      = fire_count / max_fire_count           (normalized 0..1)
//   pattern_score   = sum(pattern.frequency) / max_sum      (normalized 0..1)
//   productive_rate = productive_count / fire_count          (1.0 if no fires)
//   dormancy_mult   = if fire_count==0: >14d→0.2, >7d→0.5, else→1.0; if fires: 1.0
//   score           = (0.6*fire_score + 0.4*pattern_score) * (0.5+0.5*productive_rate) * dormancy_mult

use crate::domains;
use crate::types::{DomainCluster, Manifest, SkillInvocation};
use chrono::Utc;
use std::collections::HashMap;

// --- score_skills constants ---
const FIRE_WEIGHT: f64 = 0.6;
const PATTERN_WEIGHT: f64 = 0.4;
const PRODUCTIVE_BASE: f64 = 0.5;
const PRODUCTIVE_WEIGHT: f64 = 0.5;
const DORMANCY_SEVERE_DAYS: i64 = 14;
const DORMANCY_SEVERE_MULT: f64 = 0.2;
const DORMANCY_MODERATE_DAYS: i64 = 7;
const DORMANCY_MODERATE_MULT: f64 = 0.5;

// --- score_patterns constants ---
const PATTERN_FREQ_WEIGHT: f64 = 0.4;
const PATTERN_FIRE_WEIGHT: f64 = 0.6;

/// Score each skill based on invocation frequency, pattern richness, and productivity.
/// Returns Vec<(slug, score)> sorted by score descending.
pub fn score_skills(
    invocations: &[SkillInvocation],
    manifest: &Manifest,
    clusters: &[DomainCluster],
) -> Vec<(String, f64)> {
    if manifest.entries.is_empty() {
        return vec![];
    }

    // Group invocations by skill_name -> (total_count, productive_count)
    let mut inv_map: HashMap<&str, (usize, usize)> = HashMap::new();
    for inv in invocations {
        let entry = inv_map.entry(inv.skill_name.as_str()).or_insert((0, 0));
        entry.0 += 1;
        if inv.was_productive {
            entry.1 += 1;
        }
    }

    // Build cluster lookup: slug -> &DomainCluster
    let cluster_map: HashMap<&str, &DomainCluster> = clusters
        .iter()
        .map(|c| {
            let slug = domains::normalize(&c.domain).slug.as_str();
            // slug is &'static str from LazyLock, safe to store
            (slug, c)
        })
        .collect();

    // Build deployed_at lookup: slug -> Option<days_since_deploy>
    let deployed_map: HashMap<&str, Option<i64>> = manifest
        .entries
        .iter()
        .map(|e| {
            let days = e.deployed_at.map(|dt| (Utc::now() - dt).num_days());
            (e.slug.as_str(), days)
        })
        .collect();

    // Compute raw values per manifest entry
    struct RawScore {
        slug: String,
        fire_count: usize,
        productive_count: usize,
        pattern_freq_sum: usize,
        deployed_days: Option<i64>,
    }

    let mut raw_scores: Vec<RawScore> = Vec::with_capacity(manifest.entries.len());
    for entry in &manifest.entries {
        let fire_count = entry
            .fire_count
            .or_else(|| inv_map.get(entry.slug.as_str()).map(|(c, _)| *c))
            .unwrap_or(0);

        let productive_count = inv_map
            .get(entry.slug.as_str())
            .map(|(_, p)| *p)
            .unwrap_or(0);

        // Match entry to cluster via normalized slug
        let entry_slug = &domains::normalize(&entry.domain).slug;
        let pattern_freq_sum = cluster_map
            .get(entry_slug.as_str())
            .map(|c| c.patterns.iter().map(|p| p.frequency).sum::<usize>())
            .unwrap_or(0);

        let deployed_days = deployed_map
            .get(entry.slug.as_str())
            .copied()
            .flatten();

        raw_scores.push(RawScore {
            slug: entry.slug.clone(),
            fire_count,
            productive_count,
            pattern_freq_sum,
            deployed_days,
        });
    }

    let max_fire = raw_scores.iter().map(|r| r.fire_count).max().unwrap_or(0);
    let max_pattern = raw_scores
        .iter()
        .map(|r| r.pattern_freq_sum)
        .max()
        .unwrap_or(0);

    let mut results: Vec<(String, f64)> = raw_scores
        .into_iter()
        .map(|r| {
            let fire_score = if max_fire > 0 {
                r.fire_count as f64 / max_fire as f64
            } else {
                0.0
            };
            let pattern_score = if max_pattern > 0 {
                r.pattern_freq_sum as f64 / max_pattern as f64
            } else {
                0.0
            };
            let productive_rate = if r.fire_count > 0 {
                r.productive_count as f64 / r.fire_count as f64
            } else {
                1.0
            };
            let dormancy_multiplier = if r.fire_count == 0 {
                match r.deployed_days {
                    Some(days) if days > DORMANCY_SEVERE_DAYS => DORMANCY_SEVERE_MULT,
                    Some(days) if days > DORMANCY_MODERATE_DAYS => DORMANCY_MODERATE_MULT,
                    _ => 1.0,
                }
            } else {
                1.0
            };
            let base_score = FIRE_WEIGHT * fire_score + PATTERN_WEIGHT * pattern_score;
            let productive_multiplier = PRODUCTIVE_BASE + PRODUCTIVE_WEIGHT * productive_rate;
            let score = base_score * productive_multiplier * dormancy_multiplier;
            (r.slug, score)
        })
        .collect();

    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results
}

/// Score patterns within a cluster based on frequency and invocation overlap.
/// Returns Vec<(pattern_index, score)> sorted by score descending.
pub fn score_patterns(
    cluster: &DomainCluster,
    invocations: &[SkillInvocation],
) -> Vec<(usize, f64)> {
    if cluster.patterns.is_empty() {
        return vec![];
    }

    // Collect conversation IDs from invocations for fast lookup
    let inv_conversations: std::collections::HashSet<&str> = invocations
        .iter()
        .map(|i| i.conversation_id.as_str())
        .collect();

    let max_frequency = cluster
        .patterns
        .iter()
        .map(|p| p.frequency)
        .max()
        .unwrap_or(1);

    let mut results: Vec<(usize, f64)> = cluster
        .patterns
        .iter()
        .enumerate()
        .map(|(idx, pattern)| {
            let frequency_score = if max_frequency > 0 {
                pattern.frequency as f64 / max_frequency as f64
            } else {
                0.0
            };

            let source_fire_score = if pattern.source_ids.is_empty() {
                0.0
            } else {
                let hits = pattern
                    .source_ids
                    .iter()
                    .filter(|sid| inv_conversations.contains(sid.as_str()))
                    .count();
                hits as f64 / pattern.source_ids.len() as f64
            };

            let score = PATTERN_FREQ_WEIGHT * frequency_score + PATTERN_FIRE_WEIGHT * source_fire_score;
            (idx, score)
        })
        .collect();

    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use chrono::Utc;

    fn make_invocation(skill: &str, conv_id: &str, productive: bool) -> SkillInvocation {
        SkillInvocation {
            skill_name: skill.to_string(),
            conversation_id: conv_id.to_string(),
            timestamp: Some(Utc::now()),
            was_productive: productive,
            trigger_context: None,
        }
    }

    fn make_manifest(entries: Vec<DraftEntry>) -> Manifest {
        Manifest {
            version: "1".to_string(),
            generated_at: Utc::now(),
            entries,
            mined_ids: std::collections::HashSet::new(),
            pending_extracts: Vec::new(),
        }
    }

    fn make_entry(slug: &str, domain: &str, fire_count: Option<usize>) -> DraftEntry {
        DraftEntry {
            slug: slug.to_string(),
            domain: domain.to_string(),
            status: DraftStatus::Draft,
            pattern_count: 0,
            conversation_count: 0,
            generated_at: Utc::now(),
            deployed_at: None,
            content_hash: "abc".to_string(),
            score: None,
            fire_count,
        }
    }

    fn make_cluster(domain: &str, patterns: Vec<KnowledgePattern>) -> DomainCluster {
        DomainCluster {
            domain: domain.to_string(),
            conversations: vec![],
            patterns,
        }
    }

    fn make_pattern(frequency: usize, source_ids: Vec<&str>) -> KnowledgePattern {
        KnowledgePattern {
            title: "test".to_string(),
            description: "test".to_string(),
            steps: vec![],
            code_examples: vec![],
            source_ids: source_ids.into_iter().map(String::from).collect(),
            frequency,
            skill_slug: None,
        }
    }

    #[test]
    fn empty_inputs_return_empty() {
        let result = score_skills(&[], &make_manifest(vec![]), &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn empty_patterns_return_empty() {
        let cluster = make_cluster("Miscellaneous", vec![]);
        let result = score_patterns(&cluster, &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn skill_with_fires_scores_higher_than_without() {
        let invocations = vec![
            make_invocation("pavement", "c1", true),
            make_invocation("pavement", "c2", true),
        ];
        let manifest = make_manifest(vec![
            make_entry("pavement", "Web Development", Some(2)),
            make_entry("misc", "Miscellaneous", Some(0)),
        ]);
        let clusters = vec![
            make_cluster("Web Development", vec![make_pattern(3, vec!["c1"])]),
            make_cluster("Miscellaneous", vec![make_pattern(1, vec![])]),
        ];

        let result = score_skills(&invocations, &manifest, &clusters);
        assert_eq!(result.len(), 2);
        // pavement should be first (higher score)
        assert_eq!(result[0].0, "pavement");
        assert!(result[0].1 > result[1].1);
    }

    #[test]
    fn productive_invocations_boost_score() {
        // Two skills with same fire_count but different productivity
        let invocations_productive = vec![
            make_invocation("pavement", "c1", true),
            make_invocation("pavement", "c2", true),
        ];
        let invocations_unproductive = vec![
            make_invocation("pavement", "c1", false),
            make_invocation("pavement", "c2", false),
        ];

        let manifest = make_manifest(vec![make_entry("pavement", "Web Development", Some(2))]);
        let clusters = vec![make_cluster("Web Development", vec![make_pattern(2, vec!["c1"])])];

        let result_prod = score_skills(&invocations_productive, &manifest, &clusters);
        let result_unprod = score_skills(&invocations_unproductive, &manifest, &clusters);

        // productive_rate = 1.0 => multiplier = 1.0
        // productive_rate = 0.0 => multiplier = 0.5
        assert!(result_prod[0].1 > result_unprod[0].1);
    }

    #[test]
    fn pattern_scoring_respects_frequency() {
        let cluster = make_cluster(
            "Web Development",
            vec![
                make_pattern(1, vec!["c1"]),
                make_pattern(5, vec!["c1"]),
                make_pattern(3, vec!["c1"]),
            ],
        );
        let invocations = vec![make_invocation("pavement", "c1", true)];

        let result = score_patterns(&cluster, &invocations);
        assert_eq!(result.len(), 3);
        // Pattern index 1 (frequency=5) should be first
        assert_eq!(result[0].0, 1);
        // Pattern index 2 (frequency=3) should be second
        assert_eq!(result[1].0, 2);
        // Pattern index 0 (frequency=1) should be last
        assert_eq!(result[2].0, 0);
    }

    #[test]
    fn normalization_max_values_become_one() {
        // Single entry with fires and patterns -> all normalized values = 1.0
        let invocations = vec![make_invocation("pavement", "c1", true)];
        let manifest = make_manifest(vec![make_entry("pavement", "Web Development", Some(1))]);
        let clusters = vec![make_cluster("Web Development", vec![make_pattern(3, vec!["c1"])])];

        let result = score_skills(&invocations, &manifest, &clusters);
        assert_eq!(result.len(), 1);
        // fire_score = 1/1 = 1.0, pattern_score = 3/3 = 1.0
        // productive_rate = 1/1 = 1.0
        // score = (0.6*1.0 + 0.4*1.0) * (0.5 + 0.5*1.0) = 1.0 * 1.0 = 1.0
        assert!((result[0].1 - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn pattern_source_fire_score_works() {
        let cluster = make_cluster(
            "Web Development",
            vec![
                make_pattern(2, vec!["c1", "c2"]), // both in invocations
                make_pattern(2, vec!["c3", "c4"]), // neither in invocations
            ],
        );
        let invocations = vec![
            make_invocation("pavement", "c1", true),
            make_invocation("pavement", "c2", true),
        ];

        let result = score_patterns(&cluster, &invocations);
        assert_eq!(result.len(), 2);
        // First pattern: freq_score = 1.0, source_fire = 2/2 = 1.0 -> 0.4 + 0.6 = 1.0
        // Second pattern: freq_score = 1.0, source_fire = 0/2 = 0.0 -> 0.4 + 0.0 = 0.4
        assert_eq!(result[0].0, 0);
        assert!((result[0].1 - 1.0).abs() < f64::EPSILON);
        assert_eq!(result[1].0, 1);
        assert!((result[1].1 - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn no_fires_gives_full_productive_rate() {
        // Entry with no fire_count -> productive_rate defaults to 1.0
        let manifest = make_manifest(vec![
            make_entry("pavement", "Web Development", Some(0)),
            make_entry("misc", "Miscellaneous", Some(0)),
        ]);
        let clusters = vec![
            make_cluster("Web Development", vec![make_pattern(2, vec![])]),
            make_cluster("Miscellaneous", vec![make_pattern(1, vec![])]),
        ];

        let result = score_skills(&[], &manifest, &clusters);
        // fire_score = 0 for both, but pattern_score differs
        // productive_rate = 1.0 (no fires)
        // pavement: (0.6*0 + 0.4*1.0) * 1.0 = 0.4
        // misc:     (0.6*0 + 0.4*0.5) * 1.0 = 0.2
        assert_eq!(result[0].0, "pavement");
        assert!((result[0].1 - 0.4).abs() < f64::EPSILON);
        assert_eq!(result[1].0, "misc");
        assert!((result[1].1 - 0.2).abs() < f64::EPSILON);
    }

    fn make_entry_deployed(
        slug: &str,
        domain: &str,
        fire_count: Option<usize>,
        deployed_days_ago: i64,
    ) -> DraftEntry {
        let mut e = make_entry(slug, domain, fire_count);
        e.deployed_at = Some(Utc::now() - chrono::Duration::days(deployed_days_ago));
        e
    }

    #[test]
    fn dormancy_penalty_zero_fires_new_deploy() {
        // deployed_at = now, fire_count=0 -> no penalty (1.0 multiplier)
        let manifest = make_manifest(vec![make_entry_deployed("pavement", "Web Development", Some(0), 0)]);
        let clusters = vec![make_cluster("Web Development", vec![make_pattern(2, vec![])])];

        let result = score_skills(&[], &manifest, &clusters);
        // pattern_score = 1.0, fire_score = 0.0
        // base = 0.6*0 + 0.4*1.0 = 0.4, productive = 1.0, dormancy = 1.0
        // score = 0.4 * 1.0 * 1.0 = 0.4
        assert_eq!(result.len(), 1);
        assert!((result[0].1 - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn dormancy_penalty_zero_fires_8_days() {
        // deployed_at = 8 days ago, fire_count=0 -> 0.5 multiplier
        let manifest =
            make_manifest(vec![make_entry_deployed("pavement", "Web Development", Some(0), 8)]);
        let clusters = vec![make_cluster("Web Development", vec![make_pattern(2, vec![])])];

        let result = score_skills(&[], &manifest, &clusters);
        // base = 0.4, productive = 1.0, dormancy = 0.5
        // score = 0.4 * 1.0 * 0.5 = 0.2
        assert_eq!(result.len(), 1);
        assert!((result[0].1 - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn dormancy_penalty_zero_fires_15_days() {
        // deployed_at = 15 days ago, fire_count=0 -> 0.2 multiplier
        let manifest =
            make_manifest(vec![make_entry_deployed("pavement", "Web Development", Some(0), 15)]);
        let clusters = vec![make_cluster("Web Development", vec![make_pattern(2, vec![])])];

        let result = score_skills(&[], &manifest, &clusters);
        // base = 0.4, productive = 1.0, dormancy = 0.2
        // score = 0.4 * 1.0 * 0.2 = 0.08
        assert_eq!(result.len(), 1);
        assert!((result[0].1 - 0.08).abs() < f64::EPSILON);
    }

    #[test]
    fn dormancy_penalty_has_fires_ignores_days() {
        // deployed_at = 15 days ago, fire_count > 0 -> no penalty
        let invocations = vec![make_invocation("pavement", "c1", true)];
        let manifest =
            make_manifest(vec![make_entry_deployed("pavement", "Web Development", Some(1), 15)]);
        let clusters = vec![make_cluster("Web Development", vec![make_pattern(2, vec!["c1"])])];

        let result = score_skills(&invocations, &manifest, &clusters);
        // fire_score=1.0, pattern_score=1.0, productive_rate=1.0
        // base = 0.6*1.0 + 0.4*1.0 = 1.0, productive = 1.0, dormancy = 1.0
        // score = 1.0 * 1.0 * 1.0 = 1.0
        assert_eq!(result.len(), 1);
        assert!((result[0].1 - 1.0).abs() < f64::EPSILON);
    }
}
