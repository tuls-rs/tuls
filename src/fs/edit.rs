use similar::TextDiff;

/// A single edit operation: find `old_text`, replace with `new_text`.
#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditOperation {
    /// Text to search for - must match exactly
    pub old_text: String,
    /// Text to replace with
    pub new_text: String,
}

pub fn apply_edits(content: &str, edits: &[EditOperation]) -> Result<String, String> {
    let mut modified = content.to_owned();

    for edit in edits {
        if edit.old_text.is_empty() {
            return Err("oldText must not be empty".into());
        }
        let mut matches = modified.match_indices(&edit.old_text);
        let Some((start, _)) = matches.next() else {
            return Err(format!("oldText not found:\n{}", edit.old_text));
        };
        if matches.next().is_some() {
            return Err(format!(
                "oldText has an ambiguous match:\n{}",
                edit.old_text
            ));
        }
        modified.replace_range(start..start + edit.old_text.len(), &edit.new_text);
    }
    Ok(modified)
}

/// Render a unified diff between `original` and `modified` in git style.
pub fn render_diff(original: &str, modified: &str) -> String {
    let diff = TextDiff::from_lines(original, modified)
        .unified_diff()
        .context_radius(3)
        .header("original", "modified")
        .to_string();

    // Wrap in a fenced block, growing the backtick count if needed.
    let mut backticks = "```".to_string();
    while diff.contains(&backticks) {
        backticks.push('`');
    }
    format!("{backticks}diff\n{diff}{backticks}\n")
}

#[cfg(test)]
mod tests;
