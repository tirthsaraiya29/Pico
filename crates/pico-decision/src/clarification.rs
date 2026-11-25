//! Session-scoped clarification resolution for candidate disambiguation,
//! missing required arguments, confirmation replies, rephrasing, cancellation, and interruption.

use crate::argument::resolve_arguments;
use crate::intent::{IntentClassifier, UserIntent};
use pico_core::session::{ClarificationKind, ClarificationOption, ClarificationState};
use serde_json::{json, Value};
use tracing::debug;

/// Result of evaluating a user reply against an active session clarification state.
#[derive(Debug, Clone, PartialEq)]
pub enum ClarificationOutcome {
    /// User chose one of the candidate actions and all required arguments are satisfied.
    ExecuteAction {
        action_name: String,
        arguments: Value,
        original_request: String,
        selection_method: String,
    },
    /// User selected a candidate (or provided partial args), but required arguments remain missing.
    NeedMoreArguments {
        updated_state: ClarificationState,
        prompt: String,
    },
    /// User declined confirmation or explicitly cancelled clarification.
    Cancelled { message: String },
    /// Clarification expired due to TTL before the user answered.
    Expired {
        was_followup_reference: bool,
        message: String,
    },
    /// User changed topic with an unrelated standalone command; abandon old clarification and run new input.
    InterruptedByNewCommand,
    /// Follow-up could not unambiguously resolve the clarification; keep state and re-prompt cleanly.
    StillPending {
        updated_state: ClarificationState,
        prompt: String,
    },
}

/// Evaluates whether text looks strictly like an ordinal/confirmation reference rather than a fresh command.
pub fn looks_like_clarification_reference(input: &str) -> bool {
    let lower = input.trim().to_lowercase();
    if is_cancel_phrase(&lower) || parse_confirmation_reply(&lower).is_some() {
        return true;
    }
    parse_ordinal_choice(&lower, 10).is_some()
}

/// Resolves a user's follow-up message against a persistent `ClarificationState`.
pub fn resolve_followup(state: &ClarificationState, user_input: &str) -> ClarificationOutcome {
    let trimmed = user_input.trim();
    let lower = trimmed.to_lowercase();

    if state.is_expired() {
        return ClarificationOutcome::Expired {
            was_followup_reference: looks_like_clarification_reference(&lower),
            message: "Your previous clarification timed out. Please state your request again."
                .to_string(),
        };
    }

    if is_cancel_phrase(&lower) {
        return ClarificationOutcome::Cancelled {
            message: "Cancelled the previous action.".to_string(),
        };
    }

    match &state.kind {
        ClarificationKind::CandidateChoice => resolve_candidate_choice(state, trimmed, &lower),
        ClarificationKind::MissingArguments {
            action_name,
            parameters_schema,
            partial_arguments,
            missing_fields,
        } => resolve_missing_arguments(
            state,
            trimmed,
            &lower,
            action_name,
            parameters_schema,
            partial_arguments,
            missing_fields,
        ),
        ClarificationKind::ActionConfirmation {
            action_name,
            arguments,
            ..
        } => match parse_confirmation_reply(&lower) {
            Some(true) => ClarificationOutcome::ExecuteAction {
                action_name: action_name.clone(),
                arguments: arguments.clone(),
                original_request: state.original_request.clone(),
                selection_method: "confirmation_yes".into(),
            },
            Some(false) => ClarificationOutcome::Cancelled {
                message: format!("Cancelled '{}'.", action_name),
            },
            None => {
                if looks_like_new_standalone_command(trimmed, &state.options) {
                    return ClarificationOutcome::InterruptedByNewCommand;
                }
                let mut next = state.clone();
                next.attempts = next.attempts.saturating_add(1);
                ClarificationOutcome::StillPending {
                    prompt: format!(
                        "Please confirm whether to execute '{}': say 'yes' to proceed or 'cancel' to abort.",
                        action_name
                    ),
                    updated_state: next,
                }
            }
        },
    }
}

fn resolve_candidate_choice(
    state: &ClarificationState,
    raw_reply: &str,
    lower_reply: &str,
) -> ClarificationOutcome {
    if state.options.is_empty() {
        return ClarificationOutcome::Cancelled {
            message: "No candidate options remained available.".to_string(),
        };
    }

    // 1. Ordinal selection: "first", "the second one", "option two", "execute the first", "choose the second"
    if let Some(idx) = parse_ordinal_choice(lower_reply, state.options.len()) {
        let opt = &state.options[idx];
        return finalize_selected_option(
            state,
            opt,
            raw_reply,
            &format!("ordinal_{}", idx + 1),
        );
    }

    // 2. Explicit tool-name / suffix / distinct description keyword match
    if let Some((idx, method)) = match_explicit_option(lower_reply, &state.options) {
        let opt = &state.options[idx];
        return finalize_selected_option(state, opt, raw_reply, method);
    }

    // 3. Rephrasing or added constraint over the existing candidate pool:
    // Combine original request with reply and re-check overlap within state.options.
    if !looks_like_new_standalone_command(raw_reply, &state.options) {
        if let Some(idx) = match_constraint_hint(lower_reply, &state.options) {
            let opt = &state.options[idx];
            return finalize_selected_option(state, opt, raw_reply, "rephrase_hint");
        }
    } else {
        debug!(reply = raw_reply, "User interrupted clarification with a new command");
        return ClarificationOutcome::InterruptedByNewCommand;
    }

    // 4. Preserve state when unclear so user isn't lost on first typo/filler
    let mut next = state.clone();
    next.attempts = next.attempts.saturating_add(1);
    let options_text = format_numbered_options(&state.options);
    ClarificationOutcome::StillPending {
        prompt: format!(
            "I did not catch which option you want. Please say '1', '2', 'the first one', 'the second one', the action name, or 'cancel':\n{}",
            options_text
        ),
        updated_state: next,
    }
}

fn finalize_selected_option(
    state: &ClarificationState,
    opt: &ClarificationOption,
    reply: &str,
    method: &str,
) -> ClarificationOutcome {
    let is_clarification_ref = looks_like_clarification_reference(&reply.to_lowercase())
        || method.starts_with("ordinal_")
        || method == "explicit_tool_name";
    let combined = if is_clarification_ref {
        state.original_request.clone()
    } else {
        format!("{} {}", state.original_request, reply)
    };

    let fresh_res = resolve_arguments(&combined, &opt.action_name, &opt.parameters_schema);
    let mut merged = match opt.pre_extracted_args.clone() {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    if let Value::Object(new_map) = fresh_res.resolved_args {
        for (k, v) in new_map {
            if is_clarification_ref {
                merged.entry(k).or_insert(v);
            } else {
                merged.insert(k, v);
            }
        }
    }

    let missing = compute_missing_required(&opt.parameters_schema, &merged);
    let final_args = Value::Object(merged);

    if !missing.is_empty() {
        let prompt = format!(
            "To run '{}', please provide: {}.",
            opt.action_name,
            missing.join(", ")
        );
        let updated_state = ClarificationState {
            id: state.id.clone(),
            session_id: state.session_id,
            original_request: state.original_request.clone(),
            kind: ClarificationKind::MissingArguments {
                action_name: opt.action_name.clone(),
                parameters_schema: opt.parameters_schema.clone(),
                partial_arguments: final_args,
                missing_fields: missing.clone(),
            },
            options: state.options.clone(),
            created_at: state.created_at,
            expires_at: state.expires_at,
            attempts: 0,
        };
        return ClarificationOutcome::NeedMoreArguments {
            updated_state,
            prompt,
        };
    }

    ClarificationOutcome::ExecuteAction {
        action_name: opt.action_name.clone(),
        arguments: final_args,
        original_request: state.original_request.clone(),
        selection_method: method.to_string(),
    }
}

fn resolve_missing_arguments(
    state: &ClarificationState,
    raw_reply: &str,
    lower_reply: &str,
    action_name: &str,
    schema: &Value,
    partial_args: &Value,
    missing_fields: &[String],
) -> ClarificationOutcome {
    // Allow explicit topic switch ONLY if it starts with a completely different command
    // (e.g., "actually what is 5 * 5" or "wait check my system info")
    if looks_like_explicit_interruption(lower_reply) {
        return ClarificationOutcome::InterruptedByNewCommand;
    }

    let mut merged = match partial_args.clone() {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };

    // First run extraction on the user's fresh reply so old placeholder nouns don't stick
    let direct_res = resolve_arguments(raw_reply, action_name, schema);
    if let Value::Object(direct_map) = direct_res.resolved_args {
        for (k, v) in direct_map {
            merged.insert(k, v);
        }
    }

    // Bind concise single-slot scalar answer cleanly when needed
    let remaining_after_direct = compute_missing_required(schema, &merged);
    if remaining_after_direct.len() == 1 && !raw_reply.trim().is_empty() {
        let field = &remaining_after_direct[0];
        if let Some(parsed_val) = parse_scalar_reply_for_field(raw_reply, field, schema) {
            merged.insert(field.clone(), parsed_val);
        }
    }

    // Supplement any still-missing parameters from the combined request + reply
    if !compute_missing_required(schema, &merged).is_empty() {
        let combined = format!("{} {}", state.original_request, raw_reply);
        let combined_res = resolve_arguments(&combined, action_name, schema);
        if let Value::Object(new_map) = combined_res.resolved_args {
            for (k, v) in new_map {
                merged.entry(k).or_insert(v);
            }
        }
    }

    if missing_fields.len() == 1 && !merged.contains_key(&missing_fields[0]) {
        let field = &missing_fields[0];
        if let Some(parsed_val) = parse_scalar_reply_for_field(raw_reply, field, schema) {
            merged.insert(field.clone(), parsed_val);
        }
    }

    let still_missing = compute_missing_required(schema, &merged);
    let final_args = Value::Object(merged);

    if still_missing.is_empty() {
        ClarificationOutcome::ExecuteAction {
            action_name: action_name.to_string(),
            arguments: final_args,
            original_request: state.original_request.clone(),
            selection_method: "slot_fill".into(),
        }
    } else {
        let mut updated = state.clone();
        updated.attempts = updated.attempts.saturating_add(1);
        updated.kind = ClarificationKind::MissingArguments {
            action_name: action_name.to_string(),
            parameters_schema: schema.clone(),
            partial_arguments: final_args,
            missing_fields: still_missing.clone(),
        };
        ClarificationOutcome::NeedMoreArguments {
            updated_state: updated,
            prompt: format!(
                "I still need {} before running '{}'. Please provide {} (or say 'cancel').",
                still_missing.join(" and "),
                action_name,
                if still_missing.len() == 1 {
                    "that value"
                } else {
                    "those values"
                }
            ),
        }
    }
}

fn parse_scalar_reply_for_field(raw_reply: &str, field: &str, schema: &Value) -> Option<Value> {
    let cleaned = raw_reply
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '.' | ',' | '!' | '?'))
        .trim();
    if cleaned.is_empty() {
        return None;
    }

    let field_type = schema
        .get("properties")
        .and_then(|p| p.get(field))
        .and_then(|f| f.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("string");

    match field_type {
        "integer" => {
            if field.contains("second") || field.contains("duration") {
                if let Some(secs) = crate::argument::extract_duration_seconds(cleaned) {
                    return Some(json!(secs));
                }
            }
            crate::argument::extract_number(cleaned).map(|n| json!(n))
        }
        "number" => cleaned
            .split_whitespace()
            .next()
            .and_then(|tok| tok.trim_end_matches('%').parse::<f64>().ok())
            .map(|n| json!(n)),
        "boolean" => parse_confirmation_reply(&cleaned.to_lowercase()).map(|b| json!(b)),
        _ => {
            let trimmed_phrase = strip_slot_lead_in(cleaned);
            if trimmed_phrase.is_empty() {
                None
            } else {
                Some(json!(trimmed_phrase))
            }
        }
    }
}

fn strip_slot_lead_in(text: &str) -> &str {
    let lower = text.to_lowercase();
    for prefix in [
        "use ",
        "set it to ",
        "make it ",
        "open ",
        "launch ",
        "close ",
        "the file is ",
        "file ",
        "path ",
        "app ",
        "application ",
        "for ",
        "it is ",
        "it's ",
    ] {
        if lower.starts_with(prefix) {
            return text[prefix.len()..].trim();
        }
    }
    text
}

fn compute_missing_required(
    schema: &Value,
    args: &serde_json::Map<String, Value>,
) -> Vec<String> {
    schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter(|k| !args.contains_key(*k))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

pub fn format_numbered_options(options: &[ClarificationOption]) -> String {
    options
        .iter()
        .enumerate()
        .map(|(i, opt)| format!("{}. {} ({})", i + 1, opt.action_name, opt.description))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Matches all requested ordinal forms cleanly while rejecting unrelated sentences containing numbers.
pub fn parse_ordinal_choice(lower_input: &str, option_count: usize) -> Option<usize> {
    let tokens: Vec<String> = lower_input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect();
    if tokens.is_empty() || tokens.len() > 8 {
        return None;
    }

    let allowed_wrappers = [
        "the", "option", "choice", "action", "number", "one", "execute", "choose", "pick",
        "select", "run", "do", "use", "go", "with", "take", "try", "prefer", "want", "please",
        "i", "id", "would", "like", "let", "s",
    ];

    let mut found_idx: Option<usize> = None;
    for (pos, tok) in tokens.iter().enumerate() {
        let ordinal_idx = match tok.as_str() {
            "1" | "1st" | "first" => Some(0),
            "2" | "2nd" | "second" => Some(1),
            "3" | "3rd" | "third" => Some(2),
            "4" | "4th" | "fourth" => Some(3),
            "5" | "5th" | "fifth" => Some(4),
            "one" if pos > 0 && tokens[pos - 1] != "first" => Some(0),
            "two" => Some(1),
            "three" => Some(2),
            "four" => Some(3),
            "five" => Some(4),
            "last" if option_count > 0 => Some(option_count - 1),
            _ => None,
        };

        if let Some(idx) = ordinal_idx {
            // Note: "the first one" has "first" AND trailing "one". Keep "first" cleanly.
            if tok == "one" && tokens[..pos].iter().any(|t| matches!(t.as_str(), "first" | "second" | "third" | "fourth" | "fifth")) {
                continue;
            }
            if found_idx.is_some() && found_idx != Some(idx) {
                return None;
            }
            found_idx = Some(idx);
        } else if !allowed_wrappers.contains(&tok.as_str()) {
            return None;
        }
    }

    found_idx.filter(|&idx| idx < option_count)
}

fn match_explicit_option(
    lower_reply: &str,
    options: &[ClarificationOption],
) -> Option<(usize, &'static str)> {
    // Exact full tool name or leaf segment match
    for (idx, opt) in options.iter().enumerate() {
        let name_lower = opt.action_name.to_lowercase();
        let leaf = name_lower.rsplit('.').next().unwrap_or(&name_lower);
        let spaced = leaf.replace('_', " ");
        if lower_reply == name_lower
            || lower_reply == leaf
            || lower_reply == spaced
            || lower_reply.contains(&name_lower)
            || (spaced.len() >= 4 && lower_reply.contains(&spaced))
        {
            return Some((idx, "explicit_tool_name"));
        }
    }
    None
}

fn match_constraint_hint(lower_reply: &str, options: &[ClarificationOption]) -> Option<usize> {
    let reply_words: Vec<String> = lower_reply
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && !matches!(*w, "the" | "use" | "run" | "mode" | "action" | "one"))
        .map(String::from)
        .collect();
    if reply_words.is_empty() {
        return None;
    }

    let mut scores: Vec<(usize, usize)> = options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            let text = format!("{} {}", opt.action_name, opt.description)
                .to_lowercase()
                .replace(['.', '_'], " ");
            let hits = reply_words
                .iter()
                .filter(|w| text.split_whitespace().any(|t| t == w.as_str() || t.starts_with(w.as_str())))
                .count();
            (i, hits)
        })
        .collect();

    scores.sort_by_key(|a| std::cmp::Reverse(a.1));
    if scores[0].1 > 0 && (scores.len() == 1 || scores[0].1 > scores[1].1) {
        Some(scores[0].0)
    } else {
        None
    }
}

fn is_cancel_phrase(lower: &str) -> bool {
    let normalized = lower.trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace());
    matches!(
        normalized,
        "cancel"
            | "stop"
            | "abort"
            | "nevermind"
            | "never mind"
            | "forget it"
            | "neither"
            | "none"
            | "none of them"
            | "no thanks"
            | "skip"
    )
}

pub fn parse_confirmation_reply(lower: &str) -> Option<bool> {
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    if tokens.is_empty() {
        return None;
    }
    let has_negation = tokens.iter().any(|t| {
        matches!(
            *t,
            "no" | "nope" | "nah" | "cancel" | "abort" | "stop" | "decline" | "dont" | "don"
        )
    }) || lower.contains("don't")
        || lower.contains("do not");
    if has_negation {
        return Some(false);
    }

    let has_affirm = tokens.iter().any(|t| {
        matches!(
            *t,
            "yes" | "yeah" | "yep" | "confirm" | "confirmed" | "proceed" | "ok" | "okay" | "sure"
        )
    }) || lower.contains("go ahead")
        || lower.contains("do it");
    if has_affirm {
        Some(true)
    } else {
        None
    }
}

fn looks_like_explicit_interruption(lower: &str) -> bool {
    let prefixes = [
        "actually ",
        "instead ",
        "never mind,",
        "nevermind,",
        "wait,",
        "forget that,",
    ];
    prefixes.iter().any(|p| lower.starts_with(p))
}

fn looks_like_new_standalone_command(raw: &str, existing_options: &[ClarificationOption]) -> bool {
    let lower = raw.trim().to_lowercase();
    if looks_like_explicit_interruption(&lower) {
        return true;
    }
    let classifier = IntentClassifier::new();
    let cls = classifier.classify(raw);
    if cls.primary_intent == UserIntent::Unknown
        || cls.primary_intent == UserIntent::DirectConversation
    {
        return false;
    }
    // Check if the query already refers strongly to one of the existing option categories/terms
    if match_constraint_hint(&lower, existing_options).is_some() {
        return false;
    }
    cls.confidence >= 0.85
}

#[cfg(test)]
mod tests {
    use super::*;
    use pico_core::session::SessionId;

    fn sample_choice_state() -> ClarificationState {
        ClarificationState::new(
            SessionId::new(),
            "open my browser",
            ClarificationKind::CandidateChoice,
            vec![
                ClarificationOption {
                    action_name: "open_browser_normal".into(),
                    description: "Open browser in normal window mode".into(),
                    probability: 0.45,
                    selection_probability: 0.51,
                    parameters_schema: json!({ "type": "object", "properties": {} }),
                    pre_extracted_args: json!({}),
                    missing_required: vec![],
                },
                ClarificationOption {
                    action_name: "open_browser_private".into(),
                    description: "Open browser in private incognito mode".into(),
                    probability: 0.43,
                    selection_probability: 0.49,
                    parameters_schema: json!({ "type": "object", "properties": {} }),
                    pre_extracted_args: json!({}),
                    missing_required: vec![],
                },
            ],
            120,
        )
    }

    #[test]
    fn test_all_ordinal_selection_phrases() {
        let st = sample_choice_state();
        for phrase in ["first", "the first", "option one", "execute the first", "1"] {
            match resolve_followup(&st, phrase) {
                ClarificationOutcome::ExecuteAction { action_name, .. } => {
                    assert_eq!(action_name, "open_browser_normal", "failed phrase: {}", phrase);
                }
                other => panic!("unexpected outcome for '{}': {:?}", phrase, other),
            }
        }
        for phrase in [
            "second",
            "the second one",
            "option two",
            "the second action",
            "choose the second",
            "2",
        ] {
            match resolve_followup(&st, phrase) {
                ClarificationOutcome::ExecuteAction { action_name, .. } => {
                    assert_eq!(
                        action_name, "open_browser_private",
                        "failed phrase: {}",
                        phrase
                    );
                }
                other => panic!("unexpected outcome for '{}': {:?}", phrase, other),
            }
        }
    }

    #[test]
    fn test_rephrase_hint_and_explicit_tool_name() {
        let st = sample_choice_state();
        match resolve_followup(&st, "open it in private mode") {
            ClarificationOutcome::ExecuteAction { action_name, .. } => {
                assert_eq!(action_name, "open_browser_private");
            }
            other => panic!("unexpected: {:?}", other),
        }
        match resolve_followup(&st, "run open_browser_normal") {
            ClarificationOutcome::ExecuteAction { action_name, .. } => {
                assert_eq!(action_name, "open_browser_normal");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_unclear_reply_preserves_state() {
        let st = sample_choice_state();
        match resolve_followup(&st, "hmm um") {
            ClarificationOutcome::StillPending { updated_state, .. } => {
                assert_eq!(updated_state.attempts, 1);
                assert_eq!(updated_state.options.len(), 2);
            }
            other => panic!("expected StillPending, got {:?}", other),
        }
    }

    #[test]
    fn test_missing_argument_followup() {
        let st = ClarificationState::new(
            SessionId::new(),
            "open application",
            ClarificationKind::MissingArguments {
                action_name: "open_application".into(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": { "application": { "type": "string" } },
                    "required": ["application"]
                }),
                partial_arguments: json!({}),
                missing_fields: vec!["application".into()],
            },
            vec![],
            120,
        );
        match resolve_followup(&st, "notepad") {
            ClarificationOutcome::ExecuteAction {
                action_name,
                arguments,
                ..
            } => {
                assert_eq!(action_name, "open_application");
                assert_eq!(arguments["application"], "notepad");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_expired_state_handled() {
        let mut st = sample_choice_state();
        st.expires_at = chrono::Utc::now() - chrono::Duration::seconds(5);
        match resolve_followup(&st, "the second one") {
            ClarificationOutcome::Expired {
                was_followup_reference,
                ..
            } => assert!(was_followup_reference),
            other => panic!("expected Expired, got {:?}", other),
        }
    }
}
