use crate::compressor;
use crate::types::{ClassifiedConversation, ConversationSummary};
use anyhow::Result;
use cli_ai_analyzer::{prompt, AnalyzeOptions};

/// Classify conversation summaries into domain clusters using AI
pub fn classify(
    summaries: &[ConversationSummary],
    options: &AnalyzeOptions,
) -> Result<Vec<ClassifiedConversation>> {
    // Process in batches to stay within context limits
    let batch_size = 50;
    let mut all_classified = Vec::new();

    for batch in summaries.chunks(batch_size) {
        let text = compressor::format_for_classification(batch);
        let classified = classify_batch(batch, &text, options)?;
        all_classified.extend(classified);
    }

    Ok(all_classified)
}

fn classify_batch(
    summaries: &[ConversationSummary],
    formatted_text: &str,
    options: &AnalyzeOptions,
) -> Result<Vec<ClassifiedConversation>> {
    let prompt_text = format!(
        r#"以下はClaude Codeのチャット会話の一覧である。各会話を最も適切な分野に分類せよ。

分野の例（これに限らない、必要に応じて新しい分野を作れ）:
- 舗装工事: 施工計画、出来形管理、品質管理、温度管理、切削計算
- 写真管理: 工事写真の整理、タグ付け、台帳作成、AI解析
- PDF操作: PDF生成、結合、書き込み、テンプレート
- 施工体制: 台帳作成、下請契約、安全書類、カルテ
- スプレッドシート: Google Sheets操作、Excel操作、数式
- Rust開発: クレート設計、ビルド、テスト、WASM
- AI連携: Gemini/Claude API、プロンプト設計、精度改善
- 区画線: 数量計算、DXF、調査
- DXF/CAD: 横断図、図面生成
- 工程管理: 週報、工程表、スケジュール
- ツール設計: CLI設計、スキル作成、自動化

JSON配列で返せ。各要素: {{"index": 0, "domain": "分野名", "tags": ["tag1"], "confidence": 0.9}}

会話一覧:
{}"#,
        formatted_text
    );

    let response = prompt(&prompt_text, options.clone())?;

    // Parse JSON response
    let classifications: Vec<ClassificationEntry> = parse_json_array(&response)?;

    let mut result = Vec::new();
    for entry in classifications {
        let idx = entry.index;
        if idx < summaries.len() {
            result.push(ClassifiedConversation {
                summary: summaries[idx].clone(),
                domain: entry.domain,
                tags: entry.tags,
                confidence: entry.confidence,
            });
        }
    }

    Ok(result)
}

#[derive(serde::Deserialize)]
struct ClassificationEntry {
    index: usize,
    domain: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "default_confidence")]
    confidence: f64,
}

fn default_confidence() -> f64 {
    0.5
}

/// Sanitize AI response: remove control characters that break JSON parsing
fn sanitize_json(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\r' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// Parse a JSON array from AI response, handling markdown code fences
fn parse_json_array(response: &str) -> Result<Vec<ClassificationEntry>> {
    let sanitized = sanitize_json(response);
    let trimmed = sanitized.trim();

    // Extract JSON array from response
    let json_str = if let Some(start) = trimmed.find('[') {
        let end = trimmed.rfind(']').map(|i| i + 1).unwrap_or(trimmed.len());
        &trimmed[start..end]
    } else {
        trimmed
    };

    serde_json::from_str(json_str).map_err(|e| {
        let preview: String = response.chars().take(200).collect();
        anyhow::anyhow!("Failed to parse classification JSON: {}\nResponse: {}", e, preview)
    })
}

/// Group classified conversations by domain
pub fn group_by_domain(
    classified: &[ClassifiedConversation],
) -> std::collections::HashMap<String, Vec<&ClassifiedConversation>> {
    let mut groups = std::collections::HashMap::new();
    for c in classified {
        groups
            .entry(c.domain.clone())
            .or_insert_with(Vec::new)
            .push(c);
    }
    groups
}
