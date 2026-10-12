//! Bearer credentials for MCP clients: generation, verification and the HTTP guard.
//!
//! A token is 256 random bits from the OS CSPRNG. Only its SHA-256 verifier is ever stored;
//! the token itself is shown once when the credential is created. Tokens are accepted from the
//! `Authorization` header only — never a URL — and are never logged.

use crate::mcp::state::McpShared;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;

pub const TOKEN_PREFIX: &str = "dvmcp_";
const TOKEN_BYTES: usize = 32;

pub fn generate_token() -> Result<String, String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| format!("no secure random source: {error}"))?;
    Ok(format!("{TOKEN_PREFIX}{}", hex(&bytes)))
}

/// Lowercase hex SHA-256 of the token: what is stored in place of it.
pub fn verifier_hex(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// Constant-time comparison of two stored/presented verifiers.
pub fn verifiers_match(stored: &str, presented: &str) -> bool {
    stored.as_bytes().ct_eq(presented.as_bytes()).into()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

fn bearer_token(request: &Request) -> Option<&str> {
    let value = request
        .headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
}

fn unauthorized() -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"unauthorized"}"#,
    )
        .into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static(r#"Bearer realm="DegenView MCP""#),
    );
    response
}

/// Rejects every request that does not carry a valid, unrevoked bearer token, and hands the
/// resolved client identity to the MCP layer through the request extensions. A missing,
/// malformed, unknown or revoked token all look the same to the caller.
pub async fn require_bearer(
    State(shared): State<Arc<McpShared>>,
    mut request: Request,
    next: Next,
) -> Response {
    let identity = bearer_token(&request).and_then(|token| shared.authenticate(token));
    match identity {
        Some(identity) => {
            request.extensions_mut().insert(identity);
            next.run(request).await
        }
        None => unauthorized(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_unique_and_prefixed() {
        let a = generate_token().unwrap();
        let b = generate_token().unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with(TOKEN_PREFIX));
        assert_eq!(a.len(), TOKEN_PREFIX.len() + TOKEN_BYTES * 2);
        assert!(
            a[TOKEN_PREFIX.len()..]
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        );
    }

    #[test]
    fn verifier_is_stable_hex_sha256_and_not_the_token() {
        // SHA-256("abc")
        assert_eq!(
            verifier_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let token = generate_token().unwrap();
        assert!(!verifier_hex(&token).contains(&token));
    }

    #[test]
    fn verifier_comparison() {
        let v = verifier_hex("x");
        assert!(verifiers_match(&v, &v));
        assert!(!verifiers_match(&v, &verifier_hex("y")));
        assert!(!verifiers_match(&v, ""));
        assert!(!verifiers_match("", &v));
    }
}
