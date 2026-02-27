use crate::error::SkillMinerError;
use cli_ai_analyzer::{prompt, AnalyzeOptions};

/// Prompt template for description refinement (loaded from file at compile time).
const REFINE_PROMPT: &str = include_str!("../prompts/refine.txt");

/// Build the refinement prompt from current description and trigger contexts.
/// Exposed for unit testing.
pub fn build_refine_prompt(current_desc: &str, trigger_contexts: &[String]) -> String {
    let joined: String = trigger_contexts
        .iter()
        .map(|c| format!("- {}", c))
        .collect::<Vec<_>>()
        .join("\n");

    REFINE_PROMPT
        .replace("{current_description}", current_desc)
        .replace("{trigger_contexts}", &joined)
}

/// Use AI to refine a skill's description based on actual trigger phrases.
pub fn refine_description(
    current_desc: &str,
    trigger_contexts: &[String],
    _skill_name: &str,
    options: &AnalyzeOptions,
) -> Result<String, SkillMinerError> {
    if trigger_contexts.is_empty() {
        return Err(SkillMinerError::Config(
            "No trigger contexts available for refinement".to_string(),
        ));
    }

    let prompt_text = build_refine_prompt(current_desc, trigger_contexts);

    let response = prompt(&prompt_text, options.clone())
        .map_err(|e| SkillMinerError::Ai(e.to_string()))?;

    Ok(response.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_prompt_replaces_placeholders() {
        let desc = "Testing & QA. (test patterns) Use when user mentions: test, coverage.";
        let contexts = vec![
            "run the tests".to_string(),
            "write unit tests".to_string(),
        ];
        let result = build_refine_prompt(desc, &contexts);
        assert!(result.contains(desc));
        assert!(result.contains("- run the tests"));
        assert!(result.contains("- write unit tests"));
        assert!(!result.contains("{current_description}"));
        assert!(!result.contains("{trigger_contexts}"));
    }

    #[test]
    fn build_prompt_empty_contexts() {
        let desc = "Testing.";
        let contexts: Vec<String> = vec![];
        let result = build_refine_prompt(desc, &contexts);
        assert!(result.contains(desc));
        assert!(!result.contains("{current_description}"));
        assert!(!result.contains("{trigger_contexts}"));
    }

    #[test]
    fn refine_description_rejects_empty_contexts() {
        let options = AnalyzeOptions::default();
        let result = refine_description("test", &[], "test-skill", &options);
        assert!(result.is_err());
    }
}
