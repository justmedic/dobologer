use std::collections::HashMap;

use crate::tokenizer::{normalize_token, tokenize};

#[derive(Debug, Clone)]
pub struct ActiveBlock {
    pub id: u64,
    pub lines: Vec<String>,
    pub inverted: HashMap<String, Vec<u32>>,
    scratch: String,
}

impl ActiveBlock {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            lines: Vec::new(),
            inverted: HashMap::new(),
            scratch: String::new(),
        }
    }

    pub fn push(&mut self, line: String) {
        let line_id = self.lines.len() as u32;
        self.lines.push(line);

        let current_line = self.lines.last().expect("line just pushed");
        for token in tokenize(current_line) {
            let normalized = normalize_token(token, &mut self.scratch);
            let postings = match self.inverted.get_mut(normalized) {
                Some(postings) => postings,
                None => {
                    let key = self.scratch.clone();
                    self.inverted.entry(key).or_default()
                }
            };

            if postings.last() != Some(&line_id) {
                postings.push(line_id);
            }
        }
    }

    pub fn num_lines(&self) -> usize {
        self.lines.len()
    }

    pub fn ids_for_token(&self, token: &str) -> &[u32] {
        let mut scratch = String::new();
        let normalized = normalize_token(token, &mut scratch);
        self.inverted
            .get(normalized)
            .map(|ids| ids.as_slice())
            .unwrap_or(&[])
    }

    pub fn count_for_token(&self, token: &str) -> usize {
        self.ids_for_token(token).len()
    }

    pub fn lines_for_token(&self, token: &str, limit: usize) -> Vec<String> {
        self.ids_for_token(token)
            .iter()
            .take(limit)
            .map(|&id| self.lines[id as usize].clone())
            .collect()
    }

    pub fn search_token(&self, token: &str) -> Vec<String> {
        self.lines_for_token(token, usize::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deduplicates_tokens_within_line() {
        let mut block = ActiveBlock::new(0);
        block.push("error error error".to_string());
        let postings = block.inverted.get("error").unwrap();
        assert_eq!(postings, &vec![0]);
    }
}
