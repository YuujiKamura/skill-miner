use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A single message extracted from JSONL conversation history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub timestamp: String,
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
}

/// A parsed conversation (one session)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub source_path: PathBuf,
    pub messages: Vec<Message>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
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
    pub start_time: Option<String>,
    pub cwd: Option<String>,
    /// Key topics extracted from the conversation
    pub topics: Vec<String>,
    /// Tools used (deduplicated)
    pub tools_used: Vec<String>,
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
        }
    }
}

fn dirs_or_default() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}
