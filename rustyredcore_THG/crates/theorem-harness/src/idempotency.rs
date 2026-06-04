//! Client-provided idempotency tokens for safe retry of state-changing calls.

/// A client-provided idempotency token threaded onto every state-changing call.
///
/// The durable-execution discipline is blunt: safe retry requires the client to
/// provide a unique token stored alongside the intent, so a retried call
/// resolves to the same result instead of doing the work twice. No abstraction
/// makes a third-party action idempotent for free; the token is the mechanism.
/// This type is the token. It is threaded into
/// [`theorem_harness_core::TransitionInput`]'s `idempotency_key` on every
/// [`crate::RunHandle`] mutation.
///
/// Short-circuit ENFORCEMENT (returning the prior result for a seen token
/// instead of re-appending) is a runtime delta tracked as THPS-003: the event
/// log must persist the token on the event so a repeat can be detected. This
/// type freezes the wire shape now so that enforcement is added behind a stable
/// surface, not retrofitted after actions can fire twice.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IdempotencyToken(String);

impl IdempotencyToken {
    /// Wrap a client-provided token string.
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// The token as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the token into its owned string, for threading into a transition.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<&str> for IdempotencyToken {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for IdempotencyToken {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_token() {
        let token = IdempotencyToken::new("run-create-7");
        assert_eq!(token.as_str(), "run-create-7");
        assert_eq!(token.into_string(), "run-create-7");
    }

    #[test]
    fn equal_tokens_are_equal() {
        assert_eq!(IdempotencyToken::from("k"), IdempotencyToken::from("k"));
    }
}
