use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{1}")]
    Http(StatusCode, String),

    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Serde(#[from] serde_json::Error),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

impl AppError {
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::Http(StatusCode::NOT_FOUND, msg.into())
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::Http(StatusCode::BAD_REQUEST, msg.into())
    }

    pub fn bad_gateway(msg: impl Into<String>) -> Self {
        Self::Http(StatusCode::BAD_GATEWAY, msg.into())
    }

    #[allow(dead_code)]
    pub fn gateway_timeout(msg: impl Into<String>) -> Self {
        Self::Http(StatusCode::GATEWAY_TIMEOUT, msg.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::Http(s, m) => (*s, m.clone()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        let body = serde_json::json!({
            "error": true,
            "status": status.as_u16(),
            "message": message,
        });

        (
            status,
            [("content-type", "application/json")],
            body.to_string(),
        )
            .into_response()
    }
}
