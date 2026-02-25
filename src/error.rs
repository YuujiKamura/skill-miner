use std::fmt;

/// Typed error for skill-miner library operations.
#[derive(Debug)]
pub enum SkillMinerError {
    /// Parsing errors (JSONL, conversation structure)
    Parse(String),
    /// AI backend errors (classification, extraction)
    Ai(String),
    /// Configuration errors (missing dirs, invalid options)
    Config(String),
    /// IO errors (file read/write)
    Io(std::io::Error),
}

impl fmt::Display for SkillMinerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkillMinerError::Parse(msg) => write!(f, "parse error: {}", msg),
            SkillMinerError::Ai(msg) => write!(f, "AI error: {}", msg),
            SkillMinerError::Config(msg) => write!(f, "config error: {}", msg),
            SkillMinerError::Io(err) => write!(f, "IO error: {}", err),
        }
    }
}

impl std::error::Error for SkillMinerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SkillMinerError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SkillMinerError {
    fn from(err: std::io::Error) -> Self {
        SkillMinerError::Io(err)
    }
}

impl From<serde_json::Error> for SkillMinerError {
    fn from(err: serde_json::Error) -> Self {
        SkillMinerError::Parse(err.to_string())
    }
}
