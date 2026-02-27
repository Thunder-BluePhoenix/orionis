use axum::{
    extract::Request,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    middleware::Next,
};
use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub role: Option<String>,
}

pub async fn auth_middleware(req: Request, next: Next) -> Result<Response, StatusCode> {
    // 1. Try to get API Key (used by Agents to ingest traces)
    if let Some(auth_header) = req.headers().get("X-Orionis-Api-Key") {
        if let Ok(key_str) = auth_header.to_str() {
            let valid_key = std::env::var("ORIONIS_API_KEY").unwrap_or_else(|_| "secret-agent-key".to_string());
            if key_str == valid_key {
                return Ok(next.run(req).await);
            }
        }
    }

    // 2. Try to get JWT Bearer Token (used by Dashboard UI users)
    let auth_header = req.headers().get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok());

    let auth_header = if let Some(auth_header) = auth_header {
        auth_header
    } else {
        // Allow unauthenticated requests for now if neither is provided and auth isn't strict yet
        // In a real enterprise setup, this would return StatusCode::UNAUTHORIZED.
        // For testing, if ORIONIS_ENFORCE_AUTH is not set, we let it pass.
        if std::env::var("ORIONIS_ENFORCE_AUTH").is_ok() {
            return Err(StatusCode::UNAUTHORIZED);
        }
        return Ok(next.run(req).await);
    };

    if !auth_header.starts_with("Bearer ") {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token = &auth_header[7..];

    // JWT validation logic
    let secret = std::env::var("ORIONIS_JWT_SECRET").unwrap_or_else(|_| "secret".to_string());
    
    match decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    ) {
        Ok(_) => Ok(next.run(req).await),
        Err(_) => Err(StatusCode::UNAUTHORIZED),
    }
}
