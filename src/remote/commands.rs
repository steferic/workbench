//! Acknowledged, retryable requests from the mobile page.

use std::collections::HashMap;
use std::sync::mpsc::SyncSender;

use super::RemoteCommand;

pub type Outcome = Result<(), String>;

#[derive(Debug)]
pub struct Request {
    pub id: String,
    pub issued: i64,
    pub fingerprint: String,
    pub command: RemoteCommand,
    pub reply: SyncSender<Outcome>,
}

/// IDs are valid for ten minutes. Keep every receipt in that interval: an
/// eviction must never turn a retry into a second action.
#[derive(Debug, Default)]
pub struct Receipts(HashMap<String, (i64, String, Outcome)>);

impl Receipts {
    pub fn lookup(&mut self, request: &Request, now: i64) -> Result<Option<Outcome>, String> {
        self.0.retain(|_, (issued, _, _)| now - *issued <= 600);
        if request.issued < now - 600 || request.issued > now + 60 {
            return Err(
                "This request expired. Check the conversation before sending again.".into(),
            );
        }
        if let Some((_, fingerprint, outcome)) = self.0.get(&request.id) {
            if fingerprint != &request.fingerprint {
                return Err("This request ID was already used for a different action.".into());
            }
            return Ok(Some(outcome.clone()));
        }
        if self.0.len() >= 4096 {
            return Err("Too many recent requests. Try again shortly.".into());
        }
        Ok(None)
    }

    pub fn record(&mut self, request: &Request, outcome: Outcome) {
        self.0.insert(
            request.id.clone(),
            (request.issued, request.fingerprint.clone(), outcome),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: &str, issued: i64, text: &str) -> Request {
        Request {
            id: id.into(),
            issued,
            fingerprint: text.into(),
            command: RemoteCommand::Reply {
                agent: "a".into(),
                text: text.into(),
            },
            reply: std::sync::mpsc::sync_channel(1).0,
        }
    }

    #[test]
    fn retries_return_the_original_result_and_cannot_change_the_command() {
        let mut receipts = Receipts::default();
        let first = request("same", 1000, "hello");
        assert_eq!(receipts.lookup(&first, 1000).unwrap(), None);
        receipts.record(&first, Ok(()));
        assert_eq!(receipts.lookup(&first, 1100).unwrap(), Some(Ok(())));
        assert!(receipts
            .lookup(&request("same", 1000, "different"), 1100)
            .is_err());
        assert!(receipts.lookup(&first, 1601).is_err());
    }
}
