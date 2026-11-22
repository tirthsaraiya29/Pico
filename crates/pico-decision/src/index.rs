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
    inverted_index: HashMap<String, Vec<usize>>,
    category_index: HashMap<String, Vec<usize>>,
    doc_lengths: Vec<usize>,
    avg_doc_length: f32,
}

impl ToolIndex {
    pub fn new() -> Self {
        Self::default()
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

    /// Adds a tool entry directly to the index.
    pub fn add_entry(&mut self, entry: ToolEntry) {
        let idx = self.tools.len();
        let full_text = format!("{} {} {}", entry.name, entry.description, entry.category);
        let tokens = tokenize(&full_text);

        self.doc_lengths.push(tokens.len());
        self.category_index
            .entry(entry.category.to_lowercase())
            .or_default()
            .push(idx);

        let unique_tokens: HashSet<String> = tokens.into_iter().collect();
        for token in unique_tokens {
            self.inverted_index.entry(token).or_default().push(idx);
        }

        self.tools.push(entry);
        self.recompute_avg_doc_length();
    }

    fn recompute_avg_doc_length(&mut self) {
        if self.doc_lengths.is_empty() {
            self.avg_doc_length = 0.0;
        } else {
            let total: usize = self.doc_lengths.iter().sum();
            self.avg_doc_length = total as f32 / self.doc_lengths.len() as f32;
        }
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
                // Standard BM25 IDF formulation
                let idf = ((n_docs - df + 0.5) / (df + 0.5) + 1.0).ln();

                for &doc_idx in postings {
                    let tool = &self.tools[doc_idx];
                    // OS compatibility filter
                    if tool.os != "all" && tool.os != target_os {
                        continue;
                    }

                    let doc_len = self.doc_lengths[doc_idx] as f32;
                    let norm_len = 1.0 - B + B * (doc_len / self.avg_doc_length.max(1.0));
                    let tf = 1.5; // approximated term frequency
                    let score = idf * (tf * (K1 + 1.0)) / (tf + K1 * norm_len);

                    *scores.entry(doc_idx).or_insert(0.0) += score;
                }
            }
        }

        // 2. Category bias bonus if specified
        if let Some(pref_cat) = preferred_category {
            let pref_lower = pref_cat.to_lowercase();
            if let Some(cat_tools) = self.category_index.get(&pref_lower) {
                for &doc_idx in cat_tools {
                    let tool = &self.tools[doc_idx];
                    if tool.os == "all" || tool.os == target_os {
                        *scores.entry(doc_idx).or_insert(0.0) += 2.0;
                    }
                }
            }
        }

        // 3. Sort candidates by retrieval score
        let mut ranked_indices: Vec<(usize, f32)> = scores.into_iter().collect();
        ranked_indices.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut candidates = Vec::with_capacity(limit);
        let mut included = HashSet::new();

        for (idx, _) in ranked_indices.into_iter().take(limit) {
            included.insert(idx);
            let entry = &self.tools[idx];
            candidates.push(CandidateAction {
                name: entry.name.clone(),
                description: entry.description.clone(),
                category: entry.category.clone(),
                permission_tier: entry.permission_tier,
                parameters_schema: entry.parameters_schema.clone(),
            });
        }

        // 4. If fewer than limit, backfill with OS-compatible tools
        if candidates.len() < limit {
            for (i, entry) in self.tools.iter().enumerate() {
                if candidates.len() >= limit {
                    break;
                }
                if (entry.os == "all" || entry.os == target_os) && !included.contains(&i) {
                    candidates.push(CandidateAction {
                        name: entry.name.clone(),
                        description: entry.description.clone(),
                        category: entry.category.clone(),
                        permission_tier: entry.permission_tier,
                        parameters_schema: entry.parameters_schema.clone(),
                    });
                    included.insert(i);
                }
            }
        }

        candidates
    }

    /// Looks up a registered tool entry by name.
    pub fn get_entry(&self, name: &str) -> Option<&ToolEntry> {
        self.tools.iter().find(|t| t.name == name)
    }
}

/// Tokenizer: lowercases, splits on non-alphanumeric chars, filters stop-words and single characters.
fn tokenize(text: &str) -> Vec<String> {
    let stop_words: HashSet<&str> = [
        "a", "an", "the", "and", "or", "in", "on", "at", "to", "for", "of", "with", "by", "is",
        "are", "it", "this", "that", "which", "my", "your", "our",
    ]
    .iter()
    .cloned()
    .collect();

    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
        .filter(|t| t.len() >= 2 && !stop_words.contains(t))
        .map(|t| t.to_string())
        .collect()
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
}
