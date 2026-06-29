use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

/// Default TTL for session tokens — 7 days.
pub const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(604_800);

/// Opaque session token (typically a signed JWT string).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token(pub String);

impl Token {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Token {
    fn from(s: String) -> Self {
        Token(s)
    }
}

/// Verified user identity — used as input to `issue_token` and output from `verify_token`.
#[derive(Clone, Debug)]
pub struct UserClaims {
    pub user_id: String,
    pub email: String,
    pub tier: String,
    /// Username — empty when the identity has no username set.
    pub usuario: String,
    /// Role — empty when the identity has no role set.
    pub papel: String,
}

/// Credential variants passed to `AuthProvider::verify_credentials`.
pub enum Credentials {
    Password {
        /// Email address or username.
        identifier: String,
        password: String,
    },
    EmailMagicCode {
        email: String,
        code: String,
    },
    UatLogin {
        email: String,
        password: String,
    },
    ApiToken {
        token: String,
    },
}

/// Abstraction over authentication backends.
///
/// The default implementation is `LocalJwtProvider` (HS256 JWT signed with
/// `JWT_SECRET`). Future providers (Google OAuth, GitHub OAuth, SAML) implement
/// this trait without modifying route handlers.
#[async_trait]
pub trait AuthProvider: Send + Sync {
    /// Verify raw credentials and return identity claims.
    ///
    /// `LocalJwtProvider` does not implement this — handlers verify credentials
    /// themselves (Argon2 for passwords, `AuthStore` for magic codes) and call
    /// `issue_token` directly. Intended for OAuth providers that complete the
    /// credential exchange inline (e.g. Google token exchange in the callback).
    async fn verify_credentials(&self, creds: Credentials) -> anyhow::Result<UserClaims>;

    /// Issue a signed token for `claims` valid for `ttl`.
    async fn issue_token(&self, claims: UserClaims, ttl: Duration) -> anyhow::Result<Token>;

    /// Verify `token` and return the embedded claims.
    async fn verify_token(&self, token: &Token) -> anyhow::Result<UserClaims>;

    /// Revoke `token`. No-op for stateless JWT providers.
    async fn revoke(&self, token: &Token) -> anyhow::Result<()>;
}

/// Default auth provider: HS256 JWT signed with `JWT_SECRET` from the
/// `SecretsProvider`. Wraps the `crate::auth::Claims` JWT codec.
pub struct LocalJwtProvider {
    secrets: Arc<dyn crate::infra::secrets::SecretsProvider>,
}

impl LocalJwtProvider {
    pub fn new(secrets: Arc<dyn crate::infra::secrets::SecretsProvider>) -> Self {
        Self { secrets }
    }

    fn jwt_secret(&self) -> String {
        self.secrets
            .get("JWT_SECRET")
            .unwrap_or_else(|| "dev-secret-change-me".into())
    }
}

#[async_trait]
impl AuthProvider for LocalJwtProvider {
    async fn verify_credentials(&self, _creds: Credentials) -> anyhow::Result<UserClaims> {
        // Local credential verification requires storage access; handlers verify
        // credentials themselves and call `issue_token` directly.
        Err(anyhow::anyhow!(
            "LocalJwtProvider does not implement verify_credentials; \
             verify credentials in the handler and call issue_token directly"
        ))
    }

    async fn issue_token(&self, claims: UserClaims, ttl: Duration) -> anyhow::Result<Token> {
        use chrono::Utc;
        use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

        let now = Utc::now();
        let jwt_claims = crate::auth::Claims {
            sub: claims.user_id,
            email: claims.email,
            tier: claims.tier,
            usuario: claims.usuario,
            papel: claims.papel,
            exp: (now.timestamp() + ttl.as_secs() as i64) as usize,
            iat: now.timestamp() as usize,
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &jwt_claims,
            &EncodingKey::from_secret(self.jwt_secret().as_bytes()),
        )?;
        Ok(Token(token))
    }

    async fn verify_token(&self, token: &Token) -> anyhow::Result<UserClaims> {
        use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};

        let secret = self.jwt_secret();
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;

        let data = decode::<crate::auth::Claims>(
            token.as_str(),
            &DecodingKey::from_secret(secret.as_bytes()),
            &validation,
        )
        .map_err(|e| match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => anyhow::anyhow!("Token expired"),
            jsonwebtoken::errors::ErrorKind::InvalidSignature => {
                anyhow::anyhow!("Invalid token signature")
            }
            _ => anyhow::anyhow!("Invalid token"),
        })?;

        let c = data.claims;
        Ok(UserClaims {
            user_id: c.sub,
            email: c.email,
            tier: c.tier,
            usuario: c.usuario,
            papel: c.papel,
        })
    }

    async fn revoke(&self, _token: &Token) -> anyhow::Result<()> {
        // Stateless JWT — no revocation list.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::secrets::StaticSecretsProvider;

    fn provider() -> LocalJwtProvider {
        LocalJwtProvider::new(StaticSecretsProvider::new([("JWT_SECRET", "test-secret")]))
    }

    #[tokio::test]
    async fn test_issue_and_verify_roundtrip() {
        let p = provider();
        let claims = UserClaims {
            user_id: "user-123".into(),
            email: "test@example.com".into(),
            tier: "player".into(),
            usuario: String::new(),
            papel: String::new(),
        };
        let token = p
            .issue_token(claims, Duration::from_secs(3600))
            .await
            .unwrap();
        let verified = p.verify_token(&token).await.unwrap();
        assert_eq!(verified.user_id, "user-123");
        assert_eq!(verified.email, "test@example.com");
        assert_eq!(verified.tier, "player");
    }

    #[tokio::test]
    async fn test_verify_invalid_token_fails() {
        let p = provider();
        let result = p.verify_token(&Token("not.a.jwt".into())).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_revoke_is_noop() {
        let p = provider();
        let claims = UserClaims {
            user_id: "user-456".into(),
            email: "x@example.com".into(),
            tier: "admin".into(),
            usuario: String::new(),
            papel: String::new(),
        };
        let token = p
            .issue_token(claims, Duration::from_secs(3600))
            .await
            .unwrap();
        p.revoke(&token).await.unwrap();
        // Stateless JWT — still valid after revoke.
        p.verify_token(&token).await.unwrap();
    }

    #[tokio::test]
    async fn test_verify_credentials_not_implemented() {
        let p = provider();
        let result = p
            .verify_credentials(Credentials::Password {
                identifier: "test".into(),
                password: "pass".into(),
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_wrong_secret_fails_verification() {
        let issuer =
            LocalJwtProvider::new(StaticSecretsProvider::new([("JWT_SECRET", "secret-a")]));
        let verifier =
            LocalJwtProvider::new(StaticSecretsProvider::new([("JWT_SECRET", "secret-b")]));
        let claims = UserClaims {
            user_id: "user-789".into(),
            email: "z@example.com".into(),
            tier: "player".into(),
            usuario: String::new(),
            papel: String::new(),
        };
        let token = issuer
            .issue_token(claims, Duration::from_secs(3600))
            .await
            .unwrap();
        let result = verifier.verify_token(&token).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Invalid token signature");
    }
}
