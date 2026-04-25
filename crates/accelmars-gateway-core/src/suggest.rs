/// Compute the Levenshtein edit distance between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();

    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, val) in dp[0].iter_mut().enumerate() {
        *val = j;
    }

    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j - 1].min(dp[i - 1][j]).min(dp[i][j - 1])
            };
        }
    }
    dp[m][n]
}

/// Return up to 3 candidates from `candidates` that are similar to `input`,
/// ranked by similarity descending, filtered to similarity ≥ `threshold`.
///
/// Similarity = 1.0 - (edit_distance / max(len(input), len(candidate))).
///
/// # Lazy scan rule
/// Call this function only in error paths. Never invoke on the happy path —
/// scanning candidates on success adds latency with zero user benefit.
///
/// # Example
/// ```
/// use accelmars_gateway_core::suggest_similar;
/// let result = suggest_similar("claud", &["claude", "gemini", "groq"], 0.6);
/// assert_eq!(result, vec!["claude".to_string()]);
/// ```
pub fn suggest_similar(input: &str, candidates: &[&str], threshold: f64) -> Vec<String> {
    let mut scored: Vec<(f64, &str)> = candidates
        .iter()
        .filter_map(|&c| {
            let max_len = input.len().max(c.len());
            if max_len == 0 {
                return None;
            }
            let dist = levenshtein(input, c);
            let similarity = 1.0 - (dist as f64 / max_len as f64);
            if similarity >= threshold {
                Some((similarity, c))
            } else {
                None
            }
        })
        .collect();

    // Sort descending by similarity (higher = better match)
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(3);
    scored.into_iter().map(|(_, c)| c.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_returns_candidate() {
        let result = suggest_similar("claude", &["claude", "gemini", "groq"], 0.6);
        assert_eq!(result, vec!["claude"]);
    }

    #[test]
    fn near_match_typo_returns_candidate() {
        let result = suggest_similar("claud", &["claude", "gemini", "groq"], 0.6);
        assert_eq!(result, vec!["claude"]);
    }

    #[test]
    fn no_match_below_threshold() {
        let result = suggest_similar("completely_wrong", &["claude", "gemini"], 0.6);
        assert!(result.is_empty());
    }

    #[test]
    fn threshold_boundary_0_6() {
        // "gem" vs "gemini": edit distance 3, max_len 6, similarity = 0.5 → below threshold
        let result = suggest_similar("gem", &["gemini"], 0.6);
        assert!(
            result.is_empty(),
            "similarity 0.5 should not pass threshold 0.6"
        );
    }

    #[test]
    fn top_3_cap_on_many_matches() {
        let candidates = &[
            "claude", "claude-2", "claude-3", "claude-4", "claude-5", "claude-6",
        ];
        let result = suggest_similar("claud", candidates, 0.6);
        assert!(result.len() <= 3, "must return at most 3 suggestions");
    }

    #[test]
    fn empty_input_returns_empty() {
        let result = suggest_similar("", &["claude", "gemini"], 0.6);
        assert!(result.is_empty());
    }
}
