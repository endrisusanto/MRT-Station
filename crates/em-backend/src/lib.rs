use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use em_core::{AppError, ErrorCode, LoginCredentialsDto, Session, TokenMode};
use reqwest::{Client, StatusCode, Url};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct AuthenticatedSession {
    pub public: Session,
    pub token: SecretString,
    pub permissions: Vec<TokenMode>,
}

#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn login(
        &self,
        credentials: LoginCredentialsDto,
    ) -> Result<AuthenticatedSession, AppError>;
    async fn logout(&self, token: &SecretString) -> Result<(), AppError>;
}

#[derive(Clone)]
pub struct SimulatorAuthenticator;

#[async_trait]
impl Authenticator for SimulatorAuthenticator {
    async fn login(
        &self,
        credentials: LoginCredentialsDto,
    ) -> Result<AuthenticatedSession, AppError> {
        let username = credentials.username.trim();
        if username.is_empty() || credentials.password.is_empty() {
            return Err(AppError::new(
                ErrorCode::AuthenticationFailed,
                "Username and password are required",
            ));
        }
        let expires_at = Utc::now() + chrono::Duration::minutes(30);
        Ok(AuthenticatedSession {
            public: Session {
                user_id: username.into(),
                display_name: username.into(),
                expires_at,
                remaining_seconds: 30 * 60,
            },
            token: SecretString::from(format!("simulator:{username}")),
            permissions: simulator_permissions(),
        })
    }

    async fn logout(&self, _token: &SecretString) -> Result<(), AppError> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct HttpAuthenticator {
    client: Client,
    base_url: Url,
}

impl HttpAuthenticator {
    pub fn new(base_url: &str, timeout: Duration, allow_http: bool) -> Result<Self, AppError> {
        let base_url = Url::parse(base_url).map_err(|_| {
            AppError::new(
                ErrorCode::InvalidRequest,
                "EM_BACKEND_URL is not a valid URL",
            )
        })?;
        if base_url.scheme() != "https" && !(allow_http && base_url.scheme() == "http") {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Backend URL must use HTTPS",
            ));
        }
        let client = Client::builder()
            .timeout(timeout)
            .https_only(!allow_http)
            .build()
            .map_err(internal_error)?;
        Ok(Self { client, base_url })
    }

    fn endpoint(&self, path: &str) -> Result<Url, AppError> {
        self.base_url.join(path).map_err(internal_error)
    }
}

#[async_trait]
impl Authenticator for HttpAuthenticator {
    async fn login(
        &self,
        credentials: LoginCredentialsDto,
    ) -> Result<AuthenticatedSession, AppError> {
        let response = self
            .client
            .post(self.endpoint("v1/sessions")?)
            .json(&LoginRequest {
                username: &credentials.username,
                password: &credentials.password,
            })
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            return Err(AppError::new(
                ErrorCode::AuthenticationFailed,
                "Invalid username or password",
            ));
        }
        if status == StatusCode::FORBIDDEN {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "User is not permitted to use EM Station",
            ));
        }
        if !status.is_success() {
            return Err(AppError::new(
                ErrorCode::BackendUnavailable,
                format!("Backend returned HTTP {status}"),
            )
            .retryable());
        }
        let body: LoginResponse = response.json().await.map_err(transport_error)?;
        let remaining = (body.expires_at - Utc::now()).num_seconds();
        if remaining <= 0 || body.session_token.is_empty() {
            return Err(AppError::new(
                ErrorCode::AuthenticationFailed,
                "Backend returned an invalid session",
            ));
        }
        Ok(AuthenticatedSession {
            public: Session {
                user_id: body.user_id,
                display_name: body.display_name,
                expires_at: body.expires_at,
                remaining_seconds: remaining as u64,
            },
            token: SecretString::from(body.session_token),
            permissions: body.permissions,
        })
    }

    async fn logout(&self, token: &SecretString) -> Result<(), AppError> {
        let response = self
            .client
            .delete(self.endpoint("v1/sessions/current")?)
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(transport_error)?;
        if response.status().is_success() || response.status() == StatusCode::UNAUTHORIZED {
            return Ok(());
        }
        Err(AppError::new(
            ErrorCode::BackendUnavailable,
            format!("Backend logout returned HTTP {}", response.status()),
        )
        .retryable())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginRequest<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginResponse {
    user_id: String,
    display_name: String,
    expires_at: DateTime<Utc>,
    session_token: String,
    permissions: Vec<TokenMode>,
}

fn transport_error(error: reqwest::Error) -> AppError {
    AppError::new(ErrorCode::BackendUnavailable, error.to_string()).retryable()
}

fn internal_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::Internal, error.to_string())
}

fn simulator_permissions() -> Vec<TokenMode> {
    [
        ("MODE_ENGINEER", "Engineer", "Engineering access"),
        ("MODE_INT_EM", "Internal EM", "Internal EM access"),
        ("MODE_ACCESS_SOD", "Access SOD", "SOD access"),
        (
            "MODE_RESTRICT_JANUS",
            "Restricted Janus",
            "Restricted Janus access",
        ),
    ]
    .into_iter()
    .map(|(id, name, description)| TokenMode {
        id: id.into(),
        display_name: name.into(),
        description: description.into(),
        permitted: true,
        attributes: Default::default(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::Mutex,
    };

    #[test]
    fn requires_https_by_default() {
        let result =
            HttpAuthenticator::new("http://127.0.0.1:8000/", Duration::from_secs(1), false);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn simulator_returns_session_without_retaining_password() {
        let session = SimulatorAuthenticator
            .login(LoginCredentialsDto {
                username: "operator".into(),
                password: "temporary".into(),
            })
            .await
            .unwrap();
        assert_eq!(session.public.user_id, "operator");
        assert!(!session.permissions.is_empty());
        assert!(!session.token.expose_secret().contains("temporary"));
    }

    #[tokio::test]
    async fn http_adapter_obeys_session_contract() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let captured = requests.clone();
        let server = tokio::spawn(async move {
            for response in [
                concat!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                    r#"{"userId":"operator","displayName":"Operator","expiresAt":"2099-01-01T00:00:00Z","sessionToken":"opaque-token","permissions":[]}"#
                ),
                "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n",
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0; 8192];
                let read = stream.read(&mut buffer).await.unwrap();
                captured
                    .lock()
                    .await
                    .push(String::from_utf8_lossy(&buffer[..read]).into_owned());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let adapter = HttpAuthenticator::new(
            &format!("http://{address}/api/"),
            Duration::from_secs(2),
            true,
        )
        .unwrap();
        let session = adapter
            .login(LoginCredentialsDto {
                username: "operator".into(),
                password: "temporary".into(),
            })
            .await
            .unwrap();
        adapter.logout(&session.token).await.unwrap();
        server.await.unwrap();

        let requests = requests.lock().await;
        assert!(requests[0].starts_with("POST /api/v1/sessions HTTP/1.1"));
        assert!(requests[0].contains(r#""username":"operator""#));
        assert!(requests[1].starts_with("DELETE /api/v1/sessions/current HTTP/1.1"));
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("authorization: bearer opaque-token")
        );
    }
}
