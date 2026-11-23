use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::http::{HeaderMap, header};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const COOKIE: &str = "__Host-runlever";
pub const SESSION_LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);

pub fn new_secret() -> Result<String> {
    let mut entropy = [0u8; 32];
    getrandom::fill(&mut entropy)?;
    Ok(hex::encode(entropy))
}

/// Compare fixed-length digests so a token mismatch does not expose a matching prefix.
pub fn matches_token(candidate: &str, expected: &str) -> bool {
    Sha256::digest(candidate.as_bytes())
        .as_slice()
        .ct_eq(Sha256::digest(expected.as_bytes()).as_slice())
        .into()
}

pub fn cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|v| v.trim().split_once('='))
        .find_map(|(name, value)| (name == COOKIE).then_some(value))
}

/// Sessions expire at a fixed instant and vanish when the service restarts.
#[derive(Default)]
pub struct Sessions(Mutex<HashMap<String, Instant>>);

impl Sessions {
    pub fn issue(&self) -> Result<String> {
        let key = new_secret()?;
        let now = Instant::now();
        let mut sessions = self.0.lock().expect("session lock");
        sessions.retain(|_, end| *end > now);
        sessions.insert(key.clone(), now + SESSION_LIFETIME);
        Ok(key)
    }

    pub fn active(&self, key: &str) -> bool {
        self.0
            .lock()
            .expect("session lock")
            .get(key)
            .is_some_and(|end| *end > Instant::now())
    }

    pub fn remove(&self, key: &str) {
        self.0.lock().expect("session lock").remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_the_named_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "other=1; __Host-runlever=abc".parse().unwrap(),
        );
        assert_eq!(cookie(&headers), Some("abc"));
    }

    #[test]
    fn token_comparison_is_case_sensitive_and_checks_the_whole_value() {
        assert!(matches_token("token123", "token123"));
        assert!(!matches_token("Token123", "token123"));
        assert!(!matches_token("token", "token123"));
    }
}
