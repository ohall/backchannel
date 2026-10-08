//! Optional OAuth resource-server support. A trusted external authorization server
//! owns human authentication, consent, authorization codes, PKCE and refresh tokens.
//! Never accepts caller-selected issuers, keys, identities or signing algorithms.
use axum::{
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;

pub const SCOPE: &str = "backchannel:access";
const MAX_TOKEN_BYTES: usize = 16 * 1024;
const MAX_JWKS_BYTES: usize = 64 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(300);
const REFRESH_COOLDOWN: Duration = Duration::from_secs(30);
const MAX_TOKEN_LIFETIME: u64 = 900;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthConfig {
    pub issuer: String,
    pub jwks_uri: String,
    pub resource: String,
    /// Opt in to Auth0's exact API + issuer /userinfo audience pair for OIDC.
    /// Never permits another API audience or removes the required MCP audience.
    #[serde(default)]
    pub allow_oidc_userinfo_audience: bool,
    pub bindings: Vec<IdentityBinding>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityBinding {
    pub subject: String,
    pub client_id: String,
    pub agent_id: Uuid,
}

impl OAuthConfig {
    pub fn from_json(input: &str) -> Result<Self, String> {
        let config: Self = serde_json::from_str(input).map_err(|_| {
            "OAUTH_CONFIG must be a valid OAuth configuration JSON object".to_string()
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        let issuer = secure_url(&self.issuer)?;
        let jwks = secure_url(&self.jwks_uri)?;
        let resource = secure_url(&self.resource)?;
        if self.allow_oidc_userinfo_audience && issuer.path() != "/" {
            return Err("OIDC UserInfo audience compatibility requires a root-path issuer".into());
        }
        if issuer.origin() != jwks.origin() {
            return Err("OAuth JWKS must use the configured issuer's HTTPS origin".into());
        }
        if resource.path() != "/api/mcp" {
            return Err("OAuth resource must be the canonical HTTPS /api/mcp URL".into());
        }
        if self.bindings.is_empty() || self.bindings.len() > 1000 {
            return Err("OAuth requires 1 to 1000 explicit identity bindings".into());
        }
        let mut identities = HashSet::new();
        for binding in &self.bindings {
            if binding.subject.is_empty()
                || binding.subject.len() > 512
                || binding.client_id.is_empty()
                || binding.client_id.len() > 2048
                || binding.subject.trim() != binding.subject
                || binding.client_id.trim() != binding.client_id
                || binding.agent_id.is_nil()
                || !identities.insert((&binding.subject, &binding.client_id))
            {
                return Err(
                    "OAuth identity bindings must be nonempty, unique and have a real agent UUID"
                        .into(),
                );
            }
        }
        Ok(())
    }

    pub fn metadata_url(&self) -> String {
        let mut url = Url::parse(&self.resource).expect("validated OAuth resource");
        url.set_path("/.well-known/oauth-protected-resource/api/mcp");
        url.to_string()
    }
}

fn secure_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "OAuth URLs must be absolute HTTPS URLs")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || value.contains(['\r', '\n', '"', '\\'])
    {
        return Err("OAuth URLs must be HTTPS with no credentials, query or fragment".into());
    }
    Ok(url)
}

#[derive(Clone)]
pub struct OAuthVerifier {
    pub config: Arc<OAuthConfig>,
    client: reqwest::Client,
    cache: Arc<Mutex<KeyCache>>,
}

#[derive(Default)]
struct KeyCache {
    keys: Vec<SigningKey>,
    fetched_at: Option<Instant>,
    attempted_at: Option<Instant>,
}

#[derive(Clone, Deserialize)]
struct SigningKey {
    kid: String,
    kty: String,
    #[serde(default)]
    alg: Option<String>,
    #[serde(rename = "use", default)]
    usage: Option<String>,
    #[serde(default)]
    key_ops: Option<Vec<String>>,
    n: String,
    e: String,
}

#[derive(Deserialize)]
struct KeySet {
    keys: Vec<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct AccessClaims {
    iss: String,
    aud: Value,
    sub: String,
    client_id: String,
    jti: String,
    exp: u64,
    iat: u64,
    #[serde(default)]
    nbf: Option<u64>,
    scope: String,
}

#[derive(Debug, PartialEq)]
pub enum OAuthFailure {
    InvalidToken,
    InsufficientScope,
    Unavailable,
}

impl OAuthVerifier {
    pub fn new(config: OAuthConfig) -> Result<Self, String> {
        config.validate()?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(|_| "Could not initialize OAuth HTTPS client")?;
        Ok(Self {
            config: Arc::new(config),
            client,
            cache: Arc::new(Mutex::new(KeyCache::default())),
        })
    }

    pub async fn verify(&self, token: &str) -> Result<Uuid, OAuthFailure> {
        if token.len() > MAX_TOKEN_BYTES {
            return Err(OAuthFailure::InvalidToken);
        }
        let raw_header = token.split('.').next().ok_or(OAuthFailure::InvalidToken)?;
        let header_json: Value = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(raw_header)
                .map_err(|_| OAuthFailure::InvalidToken)?,
        )
        .map_err(|_| OAuthFailure::InvalidToken)?;
        // We implement no JOSE extensions. Never silently ignore critical ones.
        if header_json.get("crit").is_some() || header_json.get("b64").is_some() {
            return Err(OAuthFailure::InvalidToken);
        }
        let header = decode_header(token).map_err(|_| OAuthFailure::InvalidToken)?;
        if header.alg != Algorithm::RS256
            || !header.typ.as_deref().is_some_and(|typ| {
                typ.eq_ignore_ascii_case("at+jwt") || typ.eq_ignore_ascii_case("application/at+jwt")
            })
        {
            return Err(OAuthFailure::InvalidToken);
        }
        let kid = header
            .kid
            .filter(|v| !v.is_empty() && v.len() <= 256)
            .ok_or(OAuthFailure::InvalidToken)?;
        let key = self.key(&kid).await?;
        self.verify_with_key(token, &key)
    }

    fn verify_with_key(&self, token: &str, key: &DecodingKey) -> Result<Uuid, OAuthFailure> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.config.issuer]);
        validation.set_audience(&[&self.config.resource]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.validate_nbf = true;
        validation.leeway = 0;
        let claims = decode::<AccessClaims>(token, key, &validation)
            .map_err(|_| OAuthFailure::InvalidToken)?
            .claims;
        let now = jsonwebtoken::get_current_timestamp();
        if claims.exp <= now
            || claims.iat > now.saturating_add(30)
            || claims.exp <= claims.iat
            || claims.exp - claims.iat > MAX_TOKEN_LIFETIME
            || claims.sub.is_empty()
            || claims.client_id.is_empty()
            || claims.jti.is_empty()
        {
            return Err(OAuthFailure::InvalidToken);
        }
        // Default: one exact resource. Opt-in OIDC compatibility accepts only
        // that resource plus this trusted issuer's exact /userinfo audience.
        // This is not an arbitrary extra-audience allowlist.
        let audience_matches = self.audience_matches(&claims);
        if !audience_matches {
            return Err(OAuthFailure::InvalidToken);
        }
        let identity = self
            .config
            .bindings
            .iter()
            .find(|binding| binding.subject == claims.sub && binding.client_id == claims.client_id)
            .ok_or(OAuthFailure::InvalidToken)?;
        if claims.scope.bytes().any(|byte| {
            !(byte == 0x20
                || byte == 0x21
                || (0x23..=0x5b).contains(&byte)
                || (0x5d..=0x7e).contains(&byte))
        }) {
            return Err(OAuthFailure::InvalidToken);
        }
        if !claims.scope.split(' ').any(|scope| scope == SCOPE) {
            return Err(OAuthFailure::InsufficientScope);
        }
        Ok(identity.agent_id)
    }

    fn audience_matches(&self, claims: &AccessClaims) -> bool {
        if claims.aud == json!(self.config.resource) || claims.aud == json!([self.config.resource])
        {
            return true;
        }
        if !self.config.allow_oidc_userinfo_audience
            || !claims.scope.split(' ').any(|scope| scope == "openid")
        {
            return false;
        }
        let Some(audiences) = claims.aud.as_array() else {
            return false;
        };
        let Ok(mut userinfo) = Url::parse(&self.config.issuer) else {
            return false;
        };
        userinfo.set_path("/userinfo");
        audiences.len() == 2
            && audiences.contains(&json!(self.config.resource))
            && audiences.contains(&json!(userinfo.as_str()))
    }

    async fn key(&self, kid: &str) -> Result<DecodingKey, OAuthFailure> {
        // One fetch at a time, including unknown-kid refreshes. An attacker cannot
        // choose a URL or force an unbounded per-request JWKS fetch loop.
        let mut cache = self.cache.lock().await;
        let fresh = cache.fetched_at.is_some_and(|t| t.elapsed() < CACHE_TTL);
        if fresh {
            if let Some(key) = find_key(&cache.keys, kid) {
                return key;
            }
        }
        if cache
            .attempted_at
            .is_some_and(|t| t.elapsed() < REFRESH_COOLDOWN)
        {
            return Err(if fresh {
                OAuthFailure::InvalidToken
            } else {
                OAuthFailure::Unavailable
            });
        }
        cache.attempted_at = Some(Instant::now());
        let keys = self.fetch_keys().await?;
        cache.keys = keys;
        cache.fetched_at = Some(Instant::now());
        find_key(&cache.keys, kid).unwrap_or(Err(OAuthFailure::InvalidToken))
    }

    async fn fetch_keys(&self) -> Result<Vec<SigningKey>, OAuthFailure> {
        let mut response = self
            .client
            .get(&self.config.jwks_uri)
            .send()
            .await
            .map_err(|_| OAuthFailure::Unavailable)?;
        if response.status() != reqwest::StatusCode::OK
            || response
                .content_length()
                .is_some_and(|len| len > MAX_JWKS_BYTES as u64)
        {
            return Err(OAuthFailure::Unavailable);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| OAuthFailure::Unavailable)?
        {
            if bytes.len() + chunk.len() > MAX_JWKS_BYTES {
                return Err(OAuthFailure::Unavailable);
            }
            bytes.extend_from_slice(&chunk);
        }
        let set: KeySet = serde_json::from_slice(&bytes).map_err(|_| OAuthFailure::Unavailable)?;
        if set.keys.is_empty() || set.keys.len() > 32 {
            return Err(OAuthFailure::Unavailable);
        }
        let keys: Vec<SigningKey> = set
            .keys
            .into_iter()
            .filter(|value| value["kty"] == "RSA")
            .filter_map(|value| serde_json::from_value(value).ok())
            .collect();
        let mut kids = HashSet::new();
        if keys.is_empty() || keys.iter().any(|key| !kids.insert(&key.kid)) {
            return Err(OAuthFailure::Unavailable);
        }
        Ok(keys)
    }
}

fn find_key(keys: &[SigningKey], kid: &str) -> Option<Result<DecodingKey, OAuthFailure>> {
    keys.iter().find(|key| key.kid == kid).map(|key| {
        if key.kty != "RSA"
            || key.alg.as_deref().is_some_and(|alg| alg != "RS256")
            || key.usage.as_deref().is_some_and(|usage| usage != "sig")
            || key
                .key_ops
                .as_ref()
                .is_some_and(|ops| !ops.iter().any(|op| op == "verify"))
        {
            return Err(OAuthFailure::InvalidToken);
        }
        DecodingKey::from_rsa_components(&key.n, &key.e).map_err(|_| OAuthFailure::InvalidToken)
    })
}

pub async fn protected_resource_metadata(State(verifier): State<OAuthVerifier>) -> Json<Value> {
    Json(json!({
        "resource": verifier.config.resource,
        "authorization_servers": [verifier.config.issuer],
        "scopes_supported": [SCOPE],
        "bearer_methods_supported": ["header"],
        "resource_name": "Backchannel"
    }))
}

pub fn missing_credentials_challenge(config: &OAuthConfig) -> Response {
    let mut response = challenge(config, StatusCode::UNAUTHORIZED);
    let value = format!(
        "Bearer resource_metadata=\"{}\", scope=\"{}\"",
        config.metadata_url(),
        SCOPE
    );
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_str(&value).expect("validated OAuth challenge"),
    );
    response
}

pub fn challenge(config: &OAuthConfig, status: StatusCode) -> Response {
    let (error, oauth_error) = if status == StatusCode::FORBIDDEN {
        (
            "Required OAuth scope is missing",
            ", error=\"insufficient_scope\"",
        )
    } else {
        ("Authentication required", ", error=\"invalid_token\"")
    };
    let mut response = (status, Json(json!({"error": error}))).into_response();
    let value = format!(
        "Bearer resource_metadata=\"{}\", scope=\"{}\"{}",
        config.metadata_url(),
        SCOPE,
        oauth_error
    );
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_str(&value).expect("validated OAuth challenge"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};

    fn config() -> OAuthConfig {
        OAuthConfig {
            issuer: "https://id.example/".into(),
            jwks_uri: "https://id.example/jwks".into(),
            resource: "https://backchannel.example/api/mcp".into(),
            allow_oidc_userinfo_audience: false,
            bindings: vec![IdentityBinding {
                subject: "human-subject".into(),
                client_id: "chatgpt-client".into(),
                agent_id: Uuid::from_u128(1),
            }],
        }
    }
    fn claims() -> AccessClaims {
        let now = jsonwebtoken::get_current_timestamp();
        AccessClaims {
            iss: config().issuer,
            aud: json!(config().resource),
            sub: "human-subject".into(),
            client_id: "chatgpt-client".into(),
            jti: "test-token-id".into(),
            exp: now + 600,
            iat: now,
            nbf: Some(now),
            scope: SCOPE.into(),
        }
    }
    fn token(claims: &AccessClaims) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.typ = Some("at+jwt".into());
        header.kid = Some("test-key".into());
        encode(
            &header,
            claims,
            &EncodingKey::from_rsa_pem(include_bytes!("../tests/fixtures/oauth-test-private.pem"))
                .unwrap(),
        )
        .unwrap()
    }
    fn check(claims: &AccessClaims) -> Result<Uuid, OAuthFailure> {
        OAuthVerifier::new(config()).unwrap().verify_with_key(
            &token(claims),
            &DecodingKey::from_rsa_pem(include_bytes!("../tests/fixtures/oauth-test-public.pem"))
                .unwrap(),
        )
    }
    #[test]
    fn valid_exact_identity_and_single_resource() {
        assert_eq!(check(&claims()), Ok(Uuid::from_u128(1)));
        let mut value = claims();
        value.aud = json!([config().resource]);
        assert_eq!(check(&value), Ok(Uuid::from_u128(1)));
    }
    #[test]
    fn oidc_userinfo_audience_is_explicit_and_narrow() {
        let mut cfg = config();
        cfg.allow_oidc_userinfo_audience = true;
        let verifier = OAuthVerifier::new(cfg).unwrap();
        let key =
            DecodingKey::from_rsa_pem(include_bytes!("../tests/fixtures/oauth-test-public.pem"))
                .unwrap();
        let mut value = claims();
        value.scope = format!("openid profile email {SCOPE}");
        for audience in [
            json!([config().resource, "https://id.example/userinfo"]),
            json!(["https://id.example/userinfo", config().resource]),
        ] {
            value.aud = audience;
            assert_eq!(check(&value), Err(OAuthFailure::InvalidToken));
            assert_eq!(
                verifier.verify_with_key(&token(&value), &key),
                Ok(Uuid::from_u128(1))
            );
        }
        for audience in [
            json!("https://id.example/userinfo"),
            json!(["https://id.example/userinfo"]),
            json!([config().resource, config().resource]),
            json!([
                config().resource,
                "https://id.example/userinfo",
                "third-api"
            ]),
            json!([config().resource, "https://other.example/userinfo"]),
            json!([config().resource, "https://id.example/other-api"]),
            json!([config().resource, "https://id.example/userinfo/"]),
            json!([config().resource, "http://id.example/userinfo"]),
            json!([config().resource, 1]),
            json!([]),
        ] {
            value.aud = audience;
            assert_eq!(
                verifier.verify_with_key(&token(&value), &key),
                Err(OAuthFailure::InvalidToken)
            );
        }
        value.aud = json!([config().resource, "https://id.example/userinfo"]);
        for scope in [
            SCOPE.to_string(),
            format!("openid-extra {SCOPE}"),
            format!("openid\t{SCOPE}"),
        ] {
            value.scope = scope;
            assert_eq!(
                verifier.verify_with_key(&token(&value), &key),
                Err(OAuthFailure::InvalidToken)
            );
        }
        value.scope = "openid".into();
        assert_eq!(
            verifier.verify_with_key(&token(&value), &key),
            Err(OAuthFailure::InsufficientScope)
        );
        value.scope = format!("openid {SCOPE}");
        value.client_id = "wrong-client".into();
        assert_eq!(
            verifier.verify_with_key(&token(&value), &key),
            Err(OAuthFailure::InvalidToken)
        );
        value.client_id = "chatgpt-client".into();
        value.sub = "wrong-user".into();
        assert_eq!(
            verifier.verify_with_key(&token(&value), &key),
            Err(OAuthFailure::InvalidToken)
        );
    }

    #[test]
    fn oidc_userinfo_configuration_defaults_off_and_rejects_path_issuers() {
        let input = json!({
            "issuer": "https://id.example/", "jwks_uri": "https://id.example/jwks",
            "resource": config().resource,
            "bindings": [{"subject": "human-subject", "client_id": "chatgpt-client",
                          "agent_id": Uuid::from_u128(1)}]
        });
        assert!(
            !OAuthConfig::from_json(&input.to_string())
                .unwrap()
                .allow_oidc_userinfo_audience
        );
        let mut cfg = config();
        cfg.allow_oidc_userinfo_audience = true;
        cfg.issuer = "https://id.example/tenant/".into();
        assert!(cfg.validate().is_err());
        cfg.allow_oidc_userinfo_audience = false;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn rejects_bad_claims_and_unmapped_identity() {
        for mutate in [
            |c: &mut AccessClaims| c.iss = "https://attacker.example/".into(),
            |c: &mut AccessClaims| c.aud = json!("another-resource"),
            |c: &mut AccessClaims| c.aud = json!([config().resource, "another-resource"]),
            |c: &mut AccessClaims| c.sub = "other-human".into(),
            |c: &mut AccessClaims| c.client_id = "other-client".into(),
            |c: &mut AccessClaims| c.exp = jsonwebtoken::get_current_timestamp() - 10,
            |c: &mut AccessClaims| c.nbf = Some(jsonwebtoken::get_current_timestamp() + 100),
            |c: &mut AccessClaims| c.iat = jsonwebtoken::get_current_timestamp() + 100,
            |c: &mut AccessClaims| c.exp = c.iat + 901,
            |c: &mut AccessClaims| c.jti.clear(),
        ] {
            let mut value = claims();
            mutate(&mut value);
            assert_eq!(check(&value), Err(OAuthFailure::InvalidToken));
        }
    }
    #[test]
    fn scope_is_exact_not_substring() {
        for scope in ["", "backchannel:access-extra", "other"] {
            let mut value = claims();
            value.scope = scope.into();
            assert_eq!(check(&value), Err(OAuthFailure::InsufficientScope));
        }
        let mut value = claims();
        value.scope = format!("openid {SCOPE}");
        assert!(check(&value).is_ok());
    }
    #[tokio::test]
    async fn rejects_bad_header_without_network() {
        let verifier = OAuthVerifier::new(config()).unwrap();
        for value in [
            "malformed".into(),
            "x".repeat(MAX_TOKEN_BYTES + 1),
            encode(
                &Header::default(),
                &claims(),
                &EncodingKey::from_secret(b"wrong-key"),
            )
            .unwrap(),
        ] {
            assert_eq!(
                verifier.verify(&value).await,
                Err(OAuthFailure::InvalidToken)
            );
        }
        let header = Header::new(Algorithm::RS256); // ID-token-style typ=JWT
        let value = encode(
            &header,
            &claims(),
            &EncodingKey::from_rsa_pem(include_bytes!("../tests/fixtures/oauth-test-private.pem"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            verifier.verify(&value).await,
            Err(OAuthFailure::InvalidToken)
        );
    }
    #[test]
    fn rejects_insecure_or_ambiguous_configuration() {
        for mutate in [
            |c: &mut OAuthConfig| c.issuer = "http://id.example/".into(),
            |c: &mut OAuthConfig| c.jwks_uri = "https://attacker.example/jwks".into(),
            |c: &mut OAuthConfig| {
                c.resource = "https://backchannel.example/api/mcp?token=secret".into()
            },
            |c: &mut OAuthConfig| c.resource = "https://backchannel.example/v1/admin".into(),
            |c: &mut OAuthConfig| c.bindings.clear(),
            |c: &mut OAuthConfig| c.bindings.push(c.bindings[0].clone()),
            |c: &mut OAuthConfig| c.bindings[0].agent_id = Uuid::nil(),
        ] {
            let mut value = config();
            mutate(&mut value);
            assert!(value.validate().is_err());
        }
        assert!(OAuthConfig::from_json("{}").is_err());
    }
    #[tokio::test]
    async fn metadata_and_challenges_are_resource_bound() {
        let verifier = OAuthVerifier::new(config()).unwrap();
        let value = protected_resource_metadata(State(verifier)).await.0;
        assert_eq!(value["resource"], config().resource);
        assert_eq!(value["authorization_servers"], json!([config().issuer]));
        let response = challenge(&config(), StatusCode::FORBIDDEN);
        let header = response.headers()[header::WWW_AUTHENTICATE]
            .to_str()
            .unwrap();
        assert!(header.contains("error=\"insufficient_scope\""));
        assert!(header
            .contains("https://backchannel.example/.well-known/oauth-protected-resource/api/mcp"));
    }
    async fn cached_verifier() -> OAuthVerifier {
        let verifier = OAuthVerifier::new(config()).unwrap();
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/oauth-test-jwks.json")).unwrap();
        let keys: Vec<SigningKey> = serde_json::from_value(fixture["keys"].clone()).unwrap();
        *verifier.cache.lock().await = KeyCache {
            keys,
            fetched_at: Some(Instant::now()),
            attempted_at: Some(Instant::now()),
        };
        verifier
    }

    #[tokio::test]
    async fn verifies_oidc_pair_through_public_verifier() {
        let mut verifier = cached_verifier().await;
        let mut cfg = config();
        cfg.allow_oidc_userinfo_audience = true;
        verifier.config = Arc::new(cfg);
        let mut value = claims();
        for audience in [json!(config().resource), json!([config().resource])] {
            value.aud = audience;
            assert_eq!(
                verifier.verify(&token(&value)).await,
                Ok(Uuid::from_u128(1))
            );
        }
        value.aud = json!([config().resource, "https://id.example/userinfo"]);
        value.scope = format!("openid {SCOPE}");
        assert_eq!(
            verifier.verify(&token(&value)).await,
            Ok(Uuid::from_u128(1))
        );
        value.aud = json!(["other-resource", "https://id.example/userinfo"]);
        assert_eq!(
            verifier.verify(&token(&value)).await,
            Err(OAuthFailure::InvalidToken)
        );
    }

    #[tokio::test]
    async fn verifies_complete_jwt_and_rejects_unknown_kid_without_fetch() {
        let verifier = cached_verifier().await;
        assert_eq!(
            verifier.verify(&token(&claims())).await,
            Ok(Uuid::from_u128(1))
        );
        let mut header = Header::new(Algorithm::RS256);
        header.typ = Some("application/at+JWT".into());
        header.kid = Some("test-key".into());
        let key =
            EncodingKey::from_rsa_pem(include_bytes!("../tests/fixtures/oauth-test-private.pem"))
                .unwrap();
        assert!(verifier
            .verify(&encode(&header, &claims(), &key).unwrap())
            .await
            .is_ok());
        header.kid = Some("unknown-key".into());
        assert_eq!(
            verifier
                .verify(&encode(&header, &claims(), &key).unwrap())
                .await,
            Err(OAuthFailure::InvalidToken)
        );
        header.kid = Some("test-key".into());
        for name in [
            "exp",
            "iat",
            "jti",
            "sub",
            "client_id",
            "iss",
            "aud",
            "scope",
        ] {
            let mut value = serde_json::to_value(claims()).unwrap();
            value.as_object_mut().unwrap().remove(name);
            assert_eq!(
                verifier
                    .verify(&encode(&header, &value, &key).unwrap())
                    .await,
                Err(OAuthFailure::InvalidToken),
                "missing {name}"
            );
        }
        let mut value = claims();
        value.exp = jsonwebtoken::get_current_timestamp();
        value.iat = value.exp - 30;
        assert_eq!(
            verifier.verify(&token(&value)).await,
            Err(OAuthFailure::InvalidToken)
        );
        value = claims();
        value.scope = format!("openid\t{SCOPE}");
        assert_eq!(
            verifier.verify(&token(&value)).await,
            Err(OAuthFailure::InvalidToken)
        );
        let mut forged = token(&claims()).into_bytes();
        let last = forged.len() - 20;
        forged[last] = if forged[last] == b'A' { b'B' } else { b'A' };
        assert_eq!(
            verifier.verify(std::str::from_utf8(&forged).unwrap()).await,
            Err(OAuthFailure::InvalidToken)
        );
        // JOSE critical extensions must fail even before fetching keys.
        let header = URL_SAFE_NO_PAD.encode(
            br#"{"alg":"RS256","typ":"at+jwt","kid":"test-key","crit":["custom"],"custom":true}"#,
        );
        let empty_verifier = OAuthVerifier::new(config()).unwrap();
        assert_eq!(
            empty_verifier
                .verify(&format!("{header}.e30.invalid"))
                .await,
            Err(OAuthFailure::InvalidToken)
        );
        assert!(empty_verifier.cache.lock().await.attempted_at.is_none());
        let mut cache = verifier.cache.lock().await;
        cache.fetched_at = Some(Instant::now() - CACHE_TTL);
        drop(cache);
        assert_eq!(
            verifier.verify(&token(&claims())).await,
            Err(OAuthFailure::Unavailable)
        );
    }

    fn app_config(oauth: Option<OAuthConfig>) -> crate::Config {
        crate::Config {
            oauth,
            database_url: "unused".into(),
            database_schema: "backchannel_test".into(),
            viewer_token_sha256: None,
            admin_token_sha256: crate::token::hash_token("test-admin"),
            default_rate_limit_per_minute: 1000,
            admin_rate_limit_per_minute: 1000,
            max_body_size_bytes: 65536,
            max_message_body_size_bytes: 32768,
        }
    }

    #[tokio::test]
    async fn discovery_and_missing_auth_need_no_database() {
        use axum::{
            body::{to_bytes, Body},
            http::Request,
        };
        use tower::ServiceExt;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@localhost/unused")
            .unwrap();
        let router = crate::create_router(pool.clone(), app_config(None));
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/.well-known/oauth-protected-resource/api/mcp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let router = crate::create_router(pool, app_config(Some(config())));
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/.well-known/oauth-protected-resource/api/mcp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        assert_eq!(value["resource"], config().resource);
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/mcp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers()[header::WWW_AUTHENTICATE]
            .to_str()
            .unwrap()
            .contains("resource_metadata="));
    }

    /// Explicitly ignored in the fast unit suite; run with TEST_DATABASE_URL and
    /// `cargo test -p backchannel-core oauth_router_database -- --ignored`.
    #[tokio::test]
    #[ignore = "requires migrated disposable PostgreSQL database"]
    async fn oauth_router_database() {
        use axum::{
            body::{to_bytes, Body},
            http::Request,
        };
        use tower::ServiceExt;
        let pool = crate::db::create_pool(
            &std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL required"),
            "backchannel_test",
        )
        .await
        .unwrap();
        let (agent, legacy) =
            crate::db::agents::create_agent(&pool, &format!("oauth-{}", Uuid::new_v4().simple()))
                .await
                .unwrap();
        let mut verifier = cached_verifier().await;
        let mut cfg = config();
        cfg.bindings[0].agent_id = agent.id;
        cfg.allow_oidc_userinfo_audience = true;
        verifier.config = Arc::new(cfg.clone());
        let router = crate::router::create_router_with_verifier(
            pool.clone(),
            app_config(Some(cfg)),
            Some(verifier),
        );
        let jwt = token(&claims());
        let mut oidc_claims = claims();
        oidc_claims.aud = json!([config().resource, "https://id.example/userinfo"]);
        oidc_claims.scope = format!("openid {SCOPE}");
        let oidc_jwt = token(&oidc_claims);
        async fn call(
            router: &axum::Router,
            token: &str,
            path: &str,
            method: &str,
            rpc: &str,
        ) -> (StatusCode, Value) {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("Authorization", format!("Bearer {token}"))
                        .header("Content-Type", "application/json")
                        .body(Body::from(rpc.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                    .unwrap_or(Value::Null);
            (status, value)
        }
        let rpc = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"whoami","arguments":{}}}"#;
        for credential in [&jwt, &oidc_jwt, &legacy] {
            let (status, value) = call(&router, credential, "/api/mcp", "POST", rpc).await;
            assert_eq!(status, StatusCode::OK);
            let identity: Value =
                serde_json::from_str(value["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(identity["id"], agent.id.to_string());
        }
        assert_eq!(
            call(&router, &jwt, "/v1/me", "GET", "").await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(&router, &jwt, "/v1/admin/agents", "POST", "{}")
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(&router, &legacy, "/v1/me", "GET", "").await.0,
            StatusCode::OK
        );
        assert_eq!(
            call(&router, &oidc_jwt, "/v1/me", "GET", "").await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(&router, &oidc_jwt, "/v1/admin/agents", "POST", "{}")
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        let (_, value) = call(
            &router,
            &jwt,
            "/api/mcp",
            "POST",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .await;
        assert!(value["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["securitySchemes"][0]["scopes"][0] == SCOPE));
        let mut missing_scope = claims();
        missing_scope.scope.clear();
        assert_eq!(
            call(&router, &token(&missing_scope), "/api/mcp", "POST", rpc)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        let mut wrong_subject = claims();
        wrong_subject.sub = "unmapped".into();
        assert_eq!(
            call(&router, &token(&wrong_subject), "/api/mcp", "POST", rpc)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        sqlx::query("UPDATE agents SET enabled=false WHERE id=$1")
            .bind(agent.id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            call(&router, &oidc_jwt, "/api/mcp", "POST", rpc).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(&router, &jwt, "/api/mcp", "POST", rpc).await.0,
            StatusCode::UNAUTHORIZED
        );
        sqlx::query("DELETE FROM agents WHERE id=$1")
            .bind(agent.id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            call(&router, &jwt, "/api/mcp", "POST", rpc).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    #[tokio::test]
    async fn jwks_fetch_rotation_and_failure_boundaries() {
        use axum::{routing::get, Router};
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/oauth-test-jwks.json")).unwrap();
        let mut mixed = fixture.clone();
        mixed["keys"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kty":"EC","kid":"unrelated","crv":"P-256","x":"x","y":"y"}));
        let response_state = Arc::new(Mutex::new((StatusCode::OK, mixed.to_string())));
        let state = response_state.clone();
        let redirect_fixture = fixture.to_string();
        let app = Router::new()
            .route(
                "/redirect-must-not-be-followed",
                get(move || {
                    let body = redirect_fixture.clone();
                    async move { body }
                }),
            )
            .route(
                "/jwks",
                get(move || {
                    let state = state.clone();
                    async move {
                        let (status, body) = state.lock().await.clone();
                        (
                            status,
                            [(header::LOCATION, "/redirect-must-not-be-followed")],
                            body,
                        )
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut verifier = OAuthVerifier::new(config()).unwrap();
        // Test-only transport override. Production construction always validates
        // HTTPS same-origin URLs and creates an HTTPS-only no-redirect client.
        let mut local_config = config();
        local_config.jwks_uri = format!("http://{address}/jwks");
        verifier.config = Arc::new(local_config);
        verifier.client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        assert_eq!(
            verifier.verify(&token(&claims())).await,
            Ok(Uuid::from_u128(1))
        );
        let mut rotated = fixture.clone();
        rotated["keys"][0]["kid"] = json!("rotated");
        *response_state.lock().await = (StatusCode::OK, rotated.to_string());
        let mut header_value = Header::new(Algorithm::RS256);
        header_value.typ = Some("at+jwt".into());
        header_value.kid = Some("rotated".into());
        let signed = encode(
            &header_value,
            &claims(),
            &EncodingKey::from_rsa_pem(include_bytes!("../tests/fixtures/oauth-test-private.pem"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            verifier.verify(&signed).await,
            Err(OAuthFailure::InvalidToken)
        );
        verifier.cache.lock().await.attempted_at = Some(Instant::now() - REFRESH_COOLDOWN);
        assert_eq!(verifier.verify(&signed).await, Ok(Uuid::from_u128(1)));
        for (status, body) in [
            (StatusCode::FOUND, fixture.to_string()),
            (StatusCode::INTERNAL_SERVER_ERROR, fixture.to_string()),
            (StatusCode::OK, "x".repeat(MAX_JWKS_BYTES + 1)),
            (StatusCode::OK, "not-json".into()),
            (
                StatusCode::OK,
                json!({"keys": [fixture["keys"][0].clone(), fixture["keys"][0].clone()]})
                    .to_string(),
            ),
        ] {
            *response_state.lock().await = (status, body);
            assert!(matches!(
                verifier.fetch_keys().await,
                Err(OAuthFailure::Unavailable)
            ));
        }
        server.abort();
    }
}
