use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// A single message extracted from JSONL conversation history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
    /// Tool uses within this message (tool name + input summary)
    pub tool_uses: Vec<ToolUse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUse {
    pub name: String,
    /// First ~200 chars of input for context
    pub input_summary: String,
    /// File path for Edit/Read/Write tools
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    /// Command string for Bash tool (first ~100 chars)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// A parsed conversation (one session)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub source_path: PathBuf,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<DateTime<Utc>>,
    /// Working directory during this conversation
    pub cwd: Option<String>,
    /// Git branch if available
    pub git_branch: Option<String>,
}

impl Conversation {
    /// Total user+assistant messages (excluding meta)
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Quick topic summary from first user message
    pub fn first_user_message(&self) -> Option<&str> {
        self.messages
            .iter()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
    }
}

/// Compressed conversation summary for classification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: String,
    pub source_path: PathBuf,
    pub first_message: String,
    pub message_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    pub cwd: Option<String>,
    /// Key topics extracted from the conversation
    pub topics: Vec<String>,
    /// Tools used (deduplicated)
    pub tools_used: Vec<String>,
    /// File paths touched via Edit/Read/Write (deduplicated)
    #[serde(default)]
    pub files_touched: Vec<String>,
    /// Commands used via Bash (deduplicated, first ~100 chars each)
    #[serde(default)]
    pub commands_used: Vec<String>,
}

/// Domain classification result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifiedConversation {
    pub summary: ConversationSummary,
    /// Primary domain tag (e.g., "舗装工事", "写真管理", "PDF操作")
    pub domain: String,
    /// Stable English slug from domain master (e.g., "pavement", "photo-management")
    #[serde(default)]
    pub slug: String,
    /// Secondary tags
    pub tags: Vec<String>,
    /// Confidence 0.0-1.0
    pub confidence: f64,
}

/// Domain cluster: a group of conversations in the same domain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainCluster {
    pub domain: String,
    pub conversations: Vec<ClassifiedConversation>,
    /// Extracted knowledge patterns
    pub patterns: Vec<KnowledgePattern>,
}

/// A reusable knowledge pattern extracted from conversations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgePattern {
    /// What this pattern is about
    pub title: String,
    /// Detailed description
    pub description: String,
    /// Concrete steps or code examples
    pub steps: Vec<String>,
    /// Source conversation IDs
    pub source_ids: Vec<String>,
    /// How many times this pattern appeared
    pub frequency: usize,
}

/// Generated skill definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDraft {
    /// Skill name (slug)
    pub name: String,
    /// YAML description with trigger keywords
    pub description: String,
    /// Markdown body content
    pub body: String,
    /// Source patterns this skill was built from
    pub sources: Vec<String>,
    /// Whether this matches an existing skill (update vs new)
    pub existing_skill: Option<PathBuf>,
    /// Diff against existing skill if applicable
    pub diff: Option<String>,
}

// ── State management types ──

/// Status of a skill draft in the review pipeline
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DraftStatus {
    Draft,
    Approved,
    Deployed,
    Rejected,
}

impl fmt::Display for DraftStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DraftStatus::Draft => write!(f, "draft"),
            DraftStatus::Approved => write!(f, "approved"),
            DraftStatus::Deployed => write!(f, "deployed"),
            DraftStatus::Rejected => write!(f, "rejected"),
        }
    }
}

/// A single entry in the drafts manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftEntry {
    pub slug: String,
    pub domain: String,
    pub status: DraftStatus,
    pub pattern_count: usize,
    pub conversation_count: usize,
    pub generated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployed_at: Option<DateTime<Utc>>,
    pub content_hash: String,
}

/// Manifest tracking all skill drafts and their states
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub generated_at: DateTime<Utc>,
    pub entries: Vec<DraftEntry>,
}

// ── Bundle types (export/import/trading) ──

/// A portable skill bundle for sharing between environments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillBundle {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub source: BundleStats,
    pub skills: Vec<BundleSkill>,
}

/// Source statistics for a skill bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleStats {
    pub conversations: usize,
    pub domains: usize,
    pub patterns: usize,
}

/// A single skill within a bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleSkill {
    pub slug: String,
    pub domain: String,
    pub pattern_count: usize,
    pub content_hash: String,
}

/// Result of deploying a single skill
#[derive(Debug, Clone)]
pub struct DeployResult {
    pub slug: String,
    pub target_path: PathBuf,
    pub was_update: bool,
}

/// Options for pruning drafts
#[derive(Debug, Clone, Default)]
pub struct PruneOptions {
    pub duplicates: bool,
    pub misc: bool,
    pub rejected: bool,
}

/// Result of importing a bundle
#[derive(Debug, Clone)]
pub struct ImportResult {
    pub imported: Vec<String>,
    pub skipped: Vec<String>,
    pub conflicted: Vec<String>,
}

/// Statistics from a pipeline run
#[derive(Debug, Clone, Default)]
pub struct PipelineStats {
    /// Number of AI calls for classification (batch count)
    pub classify_calls: usize,
    /// Number of AI calls for extraction (domain count)
    pub extract_calls: usize,
    /// Total AI calls
    pub total_calls: usize,
}

/// Pipeline configuration
#[derive(Debug, Clone)]
pub struct MineConfig {
    /// Path to Claude Code projects directory
    pub projects_dir: PathBuf,
    /// Path to existing skills directory
    pub skills_dir: PathBuf,
    /// Path to history.jsonl
    pub history_path: PathBuf,
    /// How many days back to look
    pub days_back: u32,
    /// Minimum messages for a conversation to be included
    pub min_messages: usize,
    /// AI backend options
    pub ai_options: cli_ai_analyzer::AnalyzeOptions,
    /// Maximum parallel AI calls for extraction
    pub max_parallel: usize,
}

impl Default for MineConfig {
    fn default() -> Self {
        let home = dirs_or_default();
        Self {
            projects_dir: home.join(".claude/projects"),
            skills_dir: home.join(".claude/skills"),
            history_path: home.join(".claude/history.jsonl"),
            days_back: 30,
            min_messages: 4,
            ai_options: cli_ai_analyzer::AnalyzeOptions::default(),
            max_parallel: 4,
        }
    }
}

fn dirs_or_default() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}
