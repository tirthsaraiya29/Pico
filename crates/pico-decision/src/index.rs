//! Compact metadata index & BM25 candidate retrieval for massive tool ecosystems.
//! Enables sub-millisecond retrieval across thousands of platform-specific tools.

use pico_core::traits::{CandidateAction, PermissionTier, Tool};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Registered tool record within the compact index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEntry {
    pub name: String,
    pub description: String,
    pub category: String,
    pub os: String, // "windows", "linux", "all"
    pub permission_tier: PermissionTier,
    pub parameters_schema: serde_json::Value,
}

/// Scalable inverted index and candidate retriever.
#[derive(Debug, Clone, Default)]
pub struct ToolIndex {
    tools: Vec<ToolEntry>,
    name_to_idx: HashMap<String, usize>,
    inverted_index: HashMap<String, Vec<(usize, u32)>>,
    category_index: HashMap<String, Vec<usize>>,
    doc_lengths: Vec<usize>,
    total_doc_length: usize,
    avg_doc_length: f32,
}

impl ToolIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of indexed tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Returns true if no tools are indexed.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Registers an executable tool into the search index.
    pub fn register_tool(&mut self, tool: &Arc<dyn Tool>, category: &str, os: &str) {
        let entry = ToolEntry {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            category: category.to_string(),
            os: os.to_string(),
            permission_tier: tool.permission_tier(),
            parameters_schema: tool.parameters_schema(),
        };
        self.add_entry(entry);
    }

    /// Adds or updates a tool entry in O(tokens) time with incremental average doc length.
    pub fn add_entry(&mut self, entry: ToolEntry) {
        if let Some(&existing_idx) = self.name_to_idx.get(&entry.name) {
            self.tools[existing_idx] = entry;
            return;
        }

        let idx = self.tools.len();
        let full_text = format!("{} {} {}", entry.name, entry.description, entry.category);
        let tokens = tokenize(&full_text);
        let token_len = tokens.len();

        self.doc_lengths.push(token_len);
        self.total_doc_length += token_len;
        self.category_index
            .entry(entry.category.to_lowercase())
            .or_default()
            .push(idx);

        let mut term_counts: HashMap<String, u32> = HashMap::new();
        for token in tokens {
            *term_counts.entry(token).or_insert(0) += 1;
        }

        for (token, count) in term_counts {
            self.inverted_index
                .entry(token)
                .or_default()
                .push((idx, count));
        }

        self.name_to_idx.insert(entry.name.clone(), idx);
        self.tools.push(entry);
        self.avg_doc_length = self.total_doc_length as f32 / self.doc_lengths.len() as f32;
    }

    /// Returns the total number of registered tools in the index.
    pub fn total_tools(&self) -> usize {
        self.tools.len()
    }

    /// Retrieves candidate tools matching a user query, filtered by OS, using BM25 ranking.
    pub fn retrieve_candidates(
        &self,
        query: &str,
        target_os: &str,
        preferred_category: Option<&str>,
        limit: usize,
    ) -> Vec<CandidateAction> {
        let query_tokens = tokenize(query);
        let n_docs = self.tools.len() as f32;
        let mut scores: HashMap<usize, f32> = HashMap::new();

        const K1: f32 = 1.2;
        const B: f32 = 0.75;

        // 1. BM25 Inverted index retrieval
        for token in &query_tokens {
            if let Some(postings) = self.inverted_index.get(token) {
                let df = postings.len() as f32;
                // Standard BM25 IDF formulation: ln((N - df + 0.5) / (df + 0.5) + 1.0)
                let idf = ((n_docs - df + 0.5) / (df + 0.5) + 1.0).ln();

                for &(doc_idx, tf_count) in postings {
                    let tool = &self.tools[doc_idx];
                    // OS compatibility filter
                    if tool.os != "all" && tool.os != target_os {
                        continue;
                    }

                    let doc_len = self.doc_lengths[doc_idx] as f32;
                    let norm_len = 1.0 - B + B * (doc_len / self.avg_doc_length.max(1.0));
                    let tf = tf_count as f32;
                    let score = idf * (tf * (K1 + 1.0)) / (tf + K1 * norm_len);

                    *scores.entry(doc_idx).or_insert(0.0) += score;
                }
            }
        }

        // 2. Category bias bonus if specified
        if let Some(pref_cat) = preferred_category {
            let pref_lower = pref_cat.to_lowercase();
            let mut check_cats = vec![pref_lower.clone()];
            if pref_lower == "application_launch"
                || pref_lower == "app_launch"
                || pref_lower == "application_close"
            {
                check_cats.push("application".into());
                check_cats.push("app".into());
                if pref_lower == "application_close" {
                    check_cats.push("process".into());
                }
            } else if pref_lower == "filesystem_operation" {
                check_cats.push("filesystem".into());
                check_cats.push("fs".into());
                check_cats.push("storage".into());
            } else if pref_lower == "process_management" {
                check_cats.push("process".into());
            } else if pref_lower == "media_audio_control" {
                check_cats.push("audio".into());
            } else if pref_lower == "display_control" {
                check_cats.push("display".into());
                check_cats.push("screen".into());
            } else if pref_lower == "system_diagnostic" {
                check_cats.push("system".into());
                check_cats.push("diagnostics".into());
            } else if pref_lower == "arithmetic_computation" {
                check_cats.push("calculator".into());
                check_cats.push("math".into());
            } else if pref_lower == "timer_reminder" {
                check_cats.push("timer".into());
                check_cats.push("scheduler".into());
            } else if pref_lower == "power_management" {
                check_cats.push("power".into());
            } else if pref_lower == "clipboard_operation" {
                check_cats.push("clipboard".into());
            } else if pref_lower == "network_utility" {
                check_cats.push("network".into());
            }

            for cat in check_cats {
                if let Some(cat_tools) = self.category_index.get(&cat) {
                    for &doc_idx in cat_tools {
                        let tool = &self.tools[doc_idx];
                        if tool.os == "all" || tool.os == target_os {
                            *scores.entry(doc_idx).or_insert(0.0) += 2.5;
                        }
                    }
                }
            }
        }

        // Targeted disambiguation boosts aligned with primary intent & verbs
        let pref = preferred_category.unwrap_or("");
        if pref == "application_launch" {
            for (doc_idx, tool) in self.tools.iter().enumerate() {
                if tool.name == "open_application" {
                    *scores.entry(doc_idx).or_insert(0.0) += 6.0;
                }
            }
        } else if pref == "application_close" {
            for (doc_idx, tool) in self.tools.iter().enumerate() {
                if tool.name == "close_application" {
                    *scores.entry(doc_idx).or_insert(0.0) += 6.0;
                }
            }
        } else if pref == "display_control" {
            for (doc_idx, tool) in self.tools.iter().enumerate() {
                if tool.name == "display.set_brightness" || tool.name == "display.get_brightness" {
                    *scores.entry(doc_idx).or_insert(0.0) += 6.0;
                }
            }
        }

        if query_tokens.iter().any(|t| t == "mute" || t == "unmute") {
            for (doc_idx, tool) in self.tools.iter().enumerate() {
                if tool.name == "audio.mute" || tool.name == "mute_audio" {
                    *scores.entry(doc_idx).or_insert(0.0) += 6.0;
                }
            }
        } else if query_tokens
            .iter()
            .any(|t| t == "volume" || t == "sound" || t == "loud")
        {
            for (doc_idx, tool) in self.tools.iter().enumerate() {
                if tool.name == "audio.set_volume" || tool.name == "set_master_volume" {
                    *scores.entry(doc_idx).or_insert(0.0) += 6.0;
                }
            }
        }

        // 3. Sort candidates by retrieval score
        let mut ranked_indices: Vec<(usize, f32)> = scores.into_iter().collect();
        ranked_indices.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut candidates = Vec::with_capacity(limit);
        let mut seen_names = HashSet::new();

        for (idx, _) in ranked_indices.into_iter() {
            let entry = &self.tools[idx];
            if seen_names.insert(entry.name.clone()) {
                candidates.push(CandidateAction {
                    name: entry.name.clone(),
                    description: entry.description.clone(),
                    category: entry.category.clone(),
                    permission_tier: entry.permission_tier,
                    parameters_schema: entry.parameters_schema.clone(),
                });
                if candidates.len() >= limit {
                    break;
                }
            }
        }

        // 4. If fewer than limit, backfill with OS-compatible tools (deduplicating by name)
        if candidates.len() < limit {
            for entry in &self.tools {
                if candidates.len() >= limit {
                    break;
                }
                if (entry.os == "all" || entry.os == target_os)
                    && seen_names.insert(entry.name.clone())
                {
                    candidates.push(CandidateAction {
                        name: entry.name.clone(),
                        description: entry.description.clone(),
                        category: entry.category.clone(),
                        permission_tier: entry.permission_tier,
                        parameters_schema: entry.parameters_schema.clone(),
                    });
                }
            }
        }

        candidates
    }

    /// Looks up a registered tool entry by name in O(1).
    pub fn get_entry(&self, name: &str) -> Option<&ToolEntry> {
        self.name_to_idx.get(name).and_then(|&i| self.tools.get(i))
    }
}

/// Simple deterministic suffix stemmer for lexical retrieval normalization.
fn stem(word: &str) -> &str {
    let w = word;
    if w.len() > 5 {
        if let Some(s) = w.strip_suffix("ing") {
            return s;
        }
        if let Some(s) = w.strip_suffix("tion") {
            return s;
        }
        if let Some(s) = w.strip_suffix("ions") {
            return s;
        }
        if let Some(s) = w.strip_suffix("ment") {
            return s;
        }
    }
    if w.len() > 4 {
        if let Some(s) = w.strip_suffix("ed") {
            return s;
        }
        if let Some(s) = w.strip_suffix("er") {
            return s;
        }
        if let Some(s) = w.strip_suffix("or") {
            return s;
        }
        if let Some(s) = w.strip_suffix("es") {
            return s;
        }
    }
    if w.len() > 3 {
        if let Some(s) = w.strip_suffix('s') {
            return s;
        }
        if let Some(s) = w.strip_suffix('e') {
            return s;
        }
    }
    w
}

fn expand_index_synonym(word: &str) -> Option<&'static str> {
    match word {
        "mute" | "silence" | "quiet" => Some("mute"),
        "volume" | "sound" | "loud" | "louder" | "quieter" => Some("volume"),
        "brightness" | "backlight" | "brighter" | "dim" | "dimmer" => Some("brightness"),
        "close" | "quit" | "exit" | "terminate" | "kill" | "stop" => Some("terminate"),
        "open" | "launch" | "start" | "run" => Some("launch"),
        "calc" | "calculate" | "compute" | "math" | "evaluate" => Some("calculate"),
        "search" | "find" | "locate" => Some("search"),
        "specs" | "stats" | "hardware" | "diagnostics" => Some("system"),
        _ => None,
    }
}

/// Tokenizer: splits dotted/underscored identifiers and camelCase words, filters stop-words,
/// and produces word, stem, and synonym tokens.
fn tokenize(text: &str) -> Vec<String> {
    let stop_words: HashSet<&str> = [
        "a", "an", "the", "and", "or", "in", "on", "at", "to", "for", "of", "with", "by", "is",
        "are", "it", "this", "that", "which", "my", "your", "our",
    ]
    .iter()
    .cloned()
    .collect();

    // Split camelCase boundaries cleanly without altering all-lowercase or acronyms unnecessarily
    let mut normalized = String::with_capacity(text.len() + 8);
    let mut prev_lower = false;
    for ch in text.chars() {
        if ch.is_uppercase() && prev_lower {
            normalized.push(' ');
        }
        prev_lower = ch.is_lowercase();
        normalized.push(ch.to_ascii_lowercase());
    }

    let mut tokens = Vec::new();
    for t in normalized
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2 && !stop_words.contains(t))
    {
        tokens.push(t.to_string());
        let s = stem(t);
        if s != t && s.len() >= 3 {
            tokens.push(s.to_string());
        }
        if let Some(syn) = expand_index_synonym(t) {
            if syn != t {
                tokens.push(syn.to_string());
            }
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bm25_candidate_retrieval() {
        let mut index = ToolIndex::new();

        index.add_entry(ToolEntry {
            name: "sys.list_processes".into(),
            description: "List active processes consuming memory and CPU".into(),
            category: "process".into(),
            os: "windows".into(),
            permission_tier: PermissionTier::ReadOnly,
            parameters_schema: serde_json::json!({}),
        });

        index.add_entry(ToolEntry {
            name: "fs.read_file".into(),
            description: "Read contents of a text file from disk".into(),
            category: "filesystem".into(),
            os: "all".into(),
            permission_tier: PermissionTier::ReadOnly,
            parameters_schema: serde_json::json!({}),
        });

        index.add_entry(ToolEntry {
            name: "audio.set_volume".into(),
            description: "Set master speaker audio output volume".into(),
            category: "audio".into(),
            os: "windows".into(),
            permission_tier: PermissionTier::LowRisk,
            parameters_schema: serde_json::json!({}),
        });

        let candidates =
            index.retrieve_candidates("Which process is using RAM?", "windows", None, 2);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].name, "sys.list_processes");
    }

    #[test]
    fn test_bm25_retrieval_at_massive_scale() {
        let mut index = ToolIndex::new();
        // Register 2,000 synthetic tools spanning various domains
        for i in 0..2200 {
            let cat = match (i / 2) % 22 {
                0 => "filesystem",
                1 => "storage",
                2 => "process",
                3 => "services",
                4 => "apps",
                5 => "diagnostics",
                6 => "network_config",
                7 => "network_monitor",
                8 => "security",
                9 => "permissions",
                10 => "registry_config",
                11 => "devices",
                12 => "display",
                13 => "audio",
                14 => "power",
                15 => "scheduler",
                16 => "logs",
                17 => "packages",
                18 => "developer",
                19 => "backup",
                20 => "clipboard_shell",
                _ => "maintenance",
            };

            let os = if i % 2 == 0 { "windows" } else { "linux" };
            index.add_entry(ToolEntry {
                name: format!("{}.{}.action_{}", os, cat, i),
                description: format!(
                    "Execute action {} for {} management and diagnostic control",
                    i, cat
                ),
                category: cat.to_string(),
                os: os.to_string(),
                permission_tier: PermissionTier::LowRisk,
                parameters_schema: serde_json::json!({}),
            });
        }

        assert_eq!(index.len(), 2200);

        // Benchmark retrieval latency
        let start = std::time::Instant::now();
        let results = index.retrieve_candidates(
            "mute speaker volume audio endpoint",
            "windows",
            Some("audio"),
            5,
        );
        let duration = start.elapsed();

        assert!(!results.is_empty());
        assert!(
            duration.as_micros() < 5000,
            "BM25 retrieval across 2200 tools must be sub-5ms (expected <1ms), took {:?}",
            duration
        );
        assert!(results.iter().any(|cand| cand.category == "audio"));
    }
}
