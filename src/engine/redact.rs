use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

const MASK: &str = "[REDACTED]";

/// Matches literal credentials independently in each stream.
pub(super) struct Redactor {
    secrets: HashMap<char, Vec<String>>,
    mask: String,
}

impl Redactor {
    pub(super) fn new(secrets: Vec<String>) -> Self {
        let mut by_initial = HashMap::<_, Vec<_>>::new();
        for secret in secrets {
            if let Some(initial) = secret.chars().next() {
                by_initial.entry(initial).or_default().push(secret);
            }
        }
        for candidates in by_initial.values_mut() {
            candidates.sort_unstable_by_key(|secret| Reverse(secret.len()));
        }
        let conflicts_with_mask = by_initial
            .values()
            .flatten()
            .any(|secret| secret.contains(['[', ']']) || MASK.contains(secret.as_str()));
        let mask = if conflicts_with_mask {
            // Keep user-chosen credentials from occurring inside the marker
            // or across its boundaries.
            let used: HashSet<_> = by_initial
                .values()
                .flatten()
                .flat_map(|secret| secret.chars())
                .collect();
            ('█'..=char::MAX)
                .chain(' '..'█')
                .find(|character| !used.contains(character))
                .into_iter()
                .collect()
        } else {
            MASK.to_owned()
        };
        Self {
            secrets: by_initial,
            mask,
        }
    }

    pub(super) fn redact(&self, text: &str) -> String {
        self.redact_prefix(text, true).0
    }

    pub(super) fn redact_output(&self, pending: &mut String, eof: bool) -> String {
        let (text, consumed) = self.redact_prefix(pending, eof);
        pending.drain(..consumed);
        text
    }

    /// Emits safe text immediately and retains only a possible credential prefix.
    fn redact_prefix(&self, input: &str, eof: bool) -> (String, usize) {
        let mut text = String::with_capacity(input.len());
        let mut literal_start = 0;
        for (offset, character) in input.char_indices() {
            if offset < literal_start {
                continue;
            }
            let remaining = &input[offset..];
            let candidate = self.secrets.get(&character).and_then(|candidates| {
                candidates.iter().find(|secret| {
                    remaining.starts_with(secret.as_str())
                        || (!eof && secret.starts_with(remaining))
                })
            });
            if let Some(secret) = candidate {
                text.push_str(&input[literal_start..offset]);
                if remaining.len() < secret.len() {
                    return (text, offset);
                }
                text.push_str(&self.mask);
                literal_start = offset + secret.len();
            }
        }
        text.push_str(&input[literal_start..]);
        (text, input.len())
    }
}

#[cfg(test)]
mod tests;
