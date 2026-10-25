//! Keep secrets off the wire.
//!
//! Two layers:
//! 1. Hints password managers attach to what they copy (checked by the
//!    platform backend: [`crate::ClipboardBackend::is_marked_sensitive`]).
//! 2. Content that looks like a credential, for apps that don't set hints.
//!    This is a heuristic: expect the occasional false positive (a random
//!    product key) and false negative (a password made of words).

use std::collections::HashMap;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SensitiveReason {
    /// A password manager or the OS marked the content as sensitive.
    MarkedByApp,
    PrivateKey,
    ApiToken,
    PasswordLike,
}

impl SensitiveReason {
    pub fn describe(self) -> &'static str {
        match self {
            Self::MarkedByApp => "marked sensitive by the app that copied it",
            Self::PrivateKey => "looks like a private key",
            Self::ApiToken => "looks like an API token",
            Self::PasswordLike => "looks like a password",
        }
    }
}

/// Well-known credential prefixes.
const TOKEN_PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_", // GitHub
    "glpat-",      // GitLab
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxs-", // Slack
    "sk-",
    "sk_live_",
    "rk_live_", // OpenAI/Anthropic, Stripe
    "AIza",     // Google API keys
    "npm_",     // npm
];

pub fn detect_secret(text: &str) -> Option<SensitiveReason> {
    let t = text.trim();
    if t.contains("-----BEGIN") && t.contains("PRIVATE KEY-----") {
        return Some(SensitiveReason::PrivateKey);
    }
    if t.is_empty() || t.len() > 256 || t.chars().any(char::is_whitespace) {
        return None;
    }
    if is_token(t) {
        return Some(SensitiveReason::ApiToken);
    }
    if is_password_like(t) {
        return Some(SensitiveReason::PasswordLike);
    }
    None
}

fn is_token(t: &str) -> bool {
    let prefixed = TOKEN_PREFIXES
        .iter()
        .any(|p| t.starts_with(p) && t.len() >= p.len() + 16);
    // AWS access key ids: AKIA + 16 uppercase letters/digits.
    let aws = t.len() == 20
        && t.starts_with("AKIA")
        && t[4..]
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    prefixed || aws
}

/// A single token of 8–64 characters, mixing at least three of lowercase,
/// uppercase, digits and symbols, with high character entropy, and not
/// shaped like a URL, email address, path or number.
fn is_password_like(t: &str) -> bool {
    let len = t.chars().count();
    if !(8..=64).contains(&len) {
        return false;
    }
    if t.contains("://") || t.starts_with('/') || t.starts_with("~/") || looks_like_email(t) {
        return false;
    }
    let classes = [
        t.chars().any(|c| c.is_lowercase()),
        t.chars().any(|c| c.is_uppercase()),
        t.chars().any(|c| c.is_ascii_digit()),
        t.chars().any(|c| !c.is_alphanumeric()),
    ];
    let class_count = classes.iter().filter(|c| **c).count();
    class_count >= 3 && classes[2] && entropy_per_char(t) >= 2.8
}

fn looks_like_email(t: &str) -> bool {
    matches!(t.split_once('@'), Some((user, domain)) if !user.is_empty() && domain.contains('.'))
}

fn entropy_per_char(t: &str) -> f64 {
    let mut counts: HashMap<char, usize> = HashMap::new();
    for c in t.chars() {
        *counts.entry(c).or_default() += 1;
    }
    let n = t.chars().count() as f64;
    counts
        .values()
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_private_keys() {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----";
        assert_eq!(detect_secret(pem), Some(SensitiveReason::PrivateKey));
        let public = "-----BEGIN PUBLIC KEY-----\nMIIBIjAN\n-----END PUBLIC KEY-----";
        assert_eq!(detect_secret(public), None);
    }

    #[test]
    fn flags_api_tokens() {
        let slack_token = format!("xoxb-{}", "a".repeat(16));
        assert_eq!(
            detect_secret(&slack_token),
            Some(SensitiveReason::ApiToken),
            "Slack token"
        );

        for token in [
            "ghp_16C7e42F292c6912E7710c838347Ae178B4a",
            "github_pat_11ABCDEFG0123456789_abcdefghijklmnop",
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz012345",
            "AKIAIOSFODNN7EXAMPLE",
            "AIzaSyA1234567890abcdefghijklmnopqrstu",
        ] {
            assert_eq!(
                detect_secret(token),
                Some(SensitiveReason::ApiToken),
                "{token}"
            );
        }
    }

    #[test]
    fn flags_generated_passwords() {
        for pw in [
            "Tr0ub4dor&3",
            "x7#Kq9!mZ2pL",
            "hV9$wq2Lm!xB4zR8",
            "P@ssw0rd2026",
        ] {
            assert_eq!(
                detect_secret(pw),
                Some(SensitiveReason::PasswordLike),
                "{pw}"
            );
        }
    }

    #[test]
    fn leaves_ordinary_content_alone() {
        for text in [
            "Hello, world!",
            "Meeting moved to 3pm tomorrow",
            "https://example.com/path?query=Abc123&x=1",
            "alice.smith@example.com",
            "/home/alice/Documents/Report2026.pdf",
            "+1 (555) 123-4567",
            "1234567890",
            "README.md",
            "fn main() { println!(\"hi\"); }",
            "camelCaseVariable",
            "2026-10-25",
            "sk-short",
        ] {
            assert_eq!(detect_secret(text), None, "{text}");
        }
    }
}
