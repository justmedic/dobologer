/// Split a line into tokens without heap allocations.
/// Tokens are whitespace-separated; leading/trailing punctuation is stripped.
/// Underscores are kept inside tokens (e.g. `auth_error`).
pub fn tokenize(line: &str) -> impl Iterator<Item = &str> + '_ {
    line.split_whitespace()
        .map(trim_non_token_chars)
        .filter(|token| !token.is_empty())
}

fn is_token_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn trim_non_token_chars(word: &str) -> &str {
    let start = word
        .char_indices()
        .find(|(_, c)| is_token_char(*c))
        .map(|(i, _)| i)
        .unwrap_or(word.len());

    let end = word
        .char_indices()
        .rev()
        .find(|(_, c)| is_token_char(*c))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(start);

    &word[start..end]
}

/// Normalize a token into `scratch` (lowercase) and return the normalized slice.
pub fn normalize_token<'a>(token: &str, scratch: &'a mut String) -> &'a str {
    scratch.clear();
    scratch.extend(token.chars().flat_map(char::to_lowercase));
    scratch.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_whitespace_and_strips_punctuation() {
        let tokens: Vec<_> = tokenize("auth_error: user failed!").collect();
        assert_eq!(tokens, vec!["auth_error", "user", "failed"]);
    }
}
