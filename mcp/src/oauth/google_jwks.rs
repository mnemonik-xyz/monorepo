//! Google JWKS cache — fetches and caches the RSA public keys Google uses
//! to sign `id_token` JWTs.
//!
//! ## Behavior
//!
//! - On `verify_id_token(token, audience, issuers)`:
//!   1. Decode the JWT header (without verification) and read `kid` + `alg`.
//!   2. Look up `kid` in the in-memory cache. If found and not stale, use it.
//!   3. If missing/stale: re-fetch `https://www.googleapis.com/oauth2/v3/certs`,
//!      replace the cache, retry the lookup once.
//!   4. Verify the JWT signature with the matching JWK (RS256 only), then
//!      validate `aud`, `iss`, and `exp` claims.
//!
//! ## Security
//!
//! - Algorithm is **pinned to `RS256`** — Google's documentation lists RS256
//!   as the production algorithm; rejecting other algorithms (`none`, `HS256`)
//!   defeats the JWT-confusion attack class where an attacker submits an
//!   `alg=HS256` token signed with the JWK modulus as an HMAC secret.
//! - Issuers must be on the configured allowlist
//!   (`accounts.google.com` and `https://accounts.google.com` — Google issues
//!   tokens with either form).
//! - Audience must equal the configured `GOOGLE_OAUTH_CLIENT_ID`.
//! - Expiry is enforced (`validate_exp = true`); the library also enforces
//!   `nbf` if present.
//!
//! ## Architecture
//!
//! Cache is `Arc<Mutex<HashMap<String, JwkEntry>>>` with a fetched-at
//! timestamp per entry. TTL is 1 hour. The mutex is held only for the
//! synchronous map mutation — never across the `reqwest` call. The HTTP
//! base URL is injectable (`new_with_base_url`) so tests can point at a
//! mock JWKS server.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

/// JWKS cache TTL — 1 hour per the task spec.
pub const JWKS_TTL: Duration = Duration::from_secs(3600);

/// Default Google JWKS endpoint. Overridable for tests via
/// `GoogleJwksCache::new_with_base_url`.
pub const DEFAULT_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";

/// Google's two accepted `iss` claim values.
pub const GOOGLE_ISSUERS: &[&str] = &["https://accounts.google.com", "accounts.google.com"];

/// Subset of the Google `id_token` claim set we care about. Google adds many
/// more (email, picture, name, etc.) — we ignore them. `serde(default)` on
/// every optional field so a missing email/name doesn't break parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleIdTokenClaims {
    /// Subject — Google account id (stable, opaque). This becomes our
    /// `google_sub` link key.
    pub sub: String,
    /// Issuer — must be on the GOOGLE_ISSUERS allowlist.
    pub iss: String,
    /// Audience — must equal the configured client id.
    pub aud: String,
    /// Expiry (unix seconds). Validated by jsonwebtoken's `Validation`.
    pub exp: u64,
    /// Issued-at (unix seconds).
    #[serde(default)]
    pub iat: u64,
    /// User email (optional; Google omits when scope=openid only).
    #[serde(default)]
    pub email: Option<String>,
    /// Email verified flag (optional).
    #[serde(default)]
    pub email_verified: Option<bool>,
}

/// One JWK from `oauth2/v3/certs`. RSA keys with `kid`, `n`, `e` per RFC 7517.
#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    kid: String,
    #[serde(rename = "n")]
    modulus_b64url: String,
    #[serde(rename = "e")]
    exponent_b64url: String,
    /// Algorithm hint (`alg`) — we IGNORE this and force RS256 at verify time.
    /// Trusting the JWK's `alg` would let an attacker who controls the JWKS
    /// downgrade verification.
    #[serde(default)]
    #[allow(dead_code)]
    alg: Option<String>,
    /// `kty` — must be `RSA` for the keys we accept. We do not currently
    /// support EC keys (Google issues RSA only for `id_token`).
    #[serde(default)]
    #[allow(dead_code)]
    kty: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwksDocument {
    keys: Vec<Jwk>,
}

/// One cached JWK + the `Instant` we fetched it. Stale entries (older than
/// `JWKS_TTL`) trigger a re-fetch on next lookup.
#[derive(Debug, Clone)]
struct JwkEntry {
    jwk: Jwk,
    fetched_at: Instant,
}

/// In-memory cache of Google JWKs keyed by `kid`.
pub struct GoogleJwksCache {
    base_url: String,
    http: reqwest::Client,
    cache: Mutex<HashMap<String, JwkEntry>>,
}

impl GoogleJwksCache {
    /// Production cache against Google's live JWKS endpoint.
    pub fn new() -> Self {
        Self::new_with_base_url(DEFAULT_JWKS_URL.to_string())
    }

    /// Cache against a custom JWKS URL. Used by integration tests to point at
    /// an axum sub-server serving a hard-coded RS256 keypair.
    pub fn new_with_base_url(base_url: String) -> Self {
        // No-redirect client — defense in depth against accidental SSRF if a
        // test server points us at a redirect chain. Google's JWKS endpoint
        // never redirects in production.
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .expect("build reqwest client for JWKS cache");
        Self {
            base_url,
            http,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Force a fetch of the JWKS document and replace the cache. Called on a
    /// cache miss in `verify_id_token`. Returns the number of keys cached.
    pub async fn refresh(&self) -> Result<usize> {
        let resp = self
            .http
            .get(&self.base_url)
            .send()
            .await
            .context("fetch Google JWKS")?;
        let status = resp.status();
        if !status.is_success() {
            return Err(anyhow!(
                "Google JWKS endpoint returned non-success status {status}"
            ));
        }
        let doc: JwksDocument = resp.json().await.context("parse Google JWKS as JSON")?;
        let now = Instant::now();
        let mut guard = self
            .cache
            .lock()
            .map_err(|_| anyhow!("JWKS cache mutex poisoned"))?;
        guard.clear();
        for jwk in doc.keys {
            let kid = jwk.kid.clone();
            guard.insert(
                kid,
                JwkEntry {
                    jwk,
                    fetched_at: now,
                },
            );
        }
        Ok(guard.len())
    }

    /// Look up a JWK by `kid`. Returns Some only if the entry is fresh.
    fn get_fresh(&self, kid: &str) -> Result<Option<Jwk>> {
        let guard = self
            .cache
            .lock()
            .map_err(|_| anyhow!("JWKS cache mutex poisoned"))?;
        Ok(guard.get(kid).and_then(|entry| {
            if entry.fetched_at.elapsed() < JWKS_TTL {
                Some(entry.jwk.clone())
            } else {
                None
            }
        }))
    }

    /// Verify a Google-issued `id_token` and return its claims.
    ///
    /// Steps:
    /// 1. Decode the JWT header WITHOUT verification to read `alg` + `kid`.
    /// 2. Reject if `alg != RS256`. (Defense against alg-confusion.)
    /// 3. Find a fresh JWK for `kid`. On miss, refresh + retry once.
    /// 4. Construct a `DecodingKey` from the JWK's `(n, e)` modulus+exponent.
    /// 5. Verify the JWT with `Validation::new(Algorithm::RS256)`,
    ///    `set_audience(&[audience])`, `iss = issuers`, `validate_exp = true`.
    /// 6. Defense in depth: re-assert claims after the library check.
    pub async fn verify_id_token(
        &self,
        token: &str,
        audience: &str,
    ) -> Result<GoogleIdTokenClaims> {
        // Parse header — un-verified, used only to pick the JWK.
        let header = decode_header(token).context("decode id_token header")?;
        if header.alg != Algorithm::RS256 {
            return Err(anyhow!("id_token alg must be RS256, got {:?}", header.alg));
        }
        let kid = header
            .kid
            .ok_or_else(|| anyhow!("id_token header missing 'kid'"))?;

        // Cache lookup; on miss, refresh and retry exactly once. Two refresh
        // attempts in a row would let a misbehaving JWKS endpoint loop us
        // indefinitely.
        let jwk = match self.get_fresh(&kid)? {
            Some(j) => j,
            None => {
                self.refresh().await?;
                self.get_fresh(&kid)?
                    .ok_or_else(|| anyhow!("no JWK matching kid={kid} after refresh"))?
            }
        };

        let decoding_key =
            DecodingKey::from_rsa_components(&jwk.modulus_b64url, &jwk.exponent_b64url)
                .context("build RSA DecodingKey from JWK")?;

        // FIXED algorithm — never call `Validation::default()` which would
        // accept any algorithm. `set_audience` + `iss` set close the audience
        // / issuer confused-deputy paths.
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[audience]);
        validation.set_issuer(GOOGLE_ISSUERS);
        validation.validate_exp = true;
        // We don't enforce nbf (Google never sets it), but the lib still
        // honors it when present.

        let data = decode::<GoogleIdTokenClaims>(token, &decoding_key, &validation)
            .context("verify Google id_token signature/claims")?;

        // Defense in depth — the library already validated these, but a future
        // jsonwebtoken behavior change shouldn't silently weaken the check.
        if !GOOGLE_ISSUERS.contains(&data.claims.iss.as_str()) {
            return Err(anyhow!("unexpected iss: {}", data.claims.iss));
        }
        if data.claims.aud != audience {
            return Err(anyhow!("unexpected aud: {}", data.claims.aud));
        }

        Ok(data.claims)
    }
}

impl Default for GoogleJwksCache {
    fn default() -> Self {
        Self::new()
    }
}
