use std::collections::HashSet;

/// Compute Jaccard similarity between two strings by comparing word tokens.
pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a: HashSet<&str> = a.split_whitespace().collect();
    let set_b: HashSet<&str> = b.split_whitespace().collect();

    if set_a.is_empty() && set_b.is_empty() {
        return 1.0;
    }

    let intersection = set_a.intersection(&set_b).count() as f64;
    let union = set_a.union(&set_b).count() as f64;

    if union == 0.0 {
        return 0.0;
    }

    intersection / union
}

/// Threshold above which two memories are considered duplicates.
pub const DEDUP_THRESHOLD: f64 = 0.6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical() {
        assert_eq!(jaccard_similarity("hello world", "hello world"), 1.0);
    }

    #[test]
    fn test_completely_different() {
        assert_eq!(jaccard_similarity("hello world", "foo bar"), 0.0);
    }

    #[test]
    fn test_partial_overlap() {
        let sim = jaccard_similarity("the auth service uses JWT tokens", "auth service uses JWT for authentication");
        assert!(sim > 0.3 && sim < 0.8);
    }

    #[test]
    fn test_empty() {
        assert_eq!(jaccard_similarity("", ""), 1.0);
    }
}
