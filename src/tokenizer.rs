/// Split a line into tokens without heap allocations.
/// Tokens are separated by any character that is not alphanumeric or `_`.
/// Examples: `auth_error` stays whole, `env=prod` → `env`, `prod`.
pub fn tokenize(line: &str) -> impl Iterator<Item = &str> + '_ {
    line.split(|c: char| !is_token_char(c))
        .filter(|token| !token.is_empty())
}

fn is_token_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Normalize a token into `scratch` (lowercase) and return the normalized slice.
pub fn normalize_token<'a>(token: &str, scratch: &'a mut String) -> &'a str {
    scratch.clear();
    scratch.extend(token.chars().flat_map(char::to_lowercase));
    scratch.as_str()
}

/// Extract the first normalized token from a query string.
pub fn first_token<'a>(query: &str, scratch: &'a mut String) -> &'a str {
    tokenize(query)
        .next()
        .map(|t| normalize_token(t, scratch))
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_whitespace_and_strips_punctuation() {
        let tokens: Vec<_> = tokenize("auth_error: user failed!").collect();
        assert_eq!(tokens, vec!["auth_error", "user", "failed"]);
    }

    #[test]
    fn splits_on_equals_and_slashes() {
        let tokens: Vec<_> = tokenize("env=prod GET /api/users 200").collect();
        assert_eq!(tokens, vec!["env", "prod", "GET", "api", "users", "200"]);
    }

    #[test]
    fn keeps_underscores_inside_tokens() {
        let tokens: Vec<_> = tokenize("req_id_42 trace_id=abc").collect();
        assert_eq!(tokens, vec!["req_id_42", "trace_id", "abc"]);
    }
}
