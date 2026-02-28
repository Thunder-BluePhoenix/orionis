use axum::{
    extract::Request,
    http::{StatusCode, header},
    response::Response,
    middleware::Next,
};
use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub role: String, // "admin", "member", "viewer"
    pub tenant_id: String,
}

#[derive(Clone, Debug)]
pub struct Identity {
    pub user_id: String,
    pub tenant_id: String,
    pub role: String,
}

pub async fn auth_middleware(mut req: Request, next: Next) -> Result<Response, StatusCode> {
    // 1. Try to get API Key (used by Agents to ingest traces)
    let api_key_check = req.headers().get("X-Orionis-Api-Key")
        .and_then(|h| h.to_str().ok())
        .map(|k| k == std::env::var("ORIONIS_API_KEY").unwrap_or_else(|_| "secret-agent-key".to_string()))
        .unwrap_or(false);

    if api_key_check {
        let tenant_id = req.headers().get("X-Tenant-Id")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("default")
            .to_string();
        
        req.extensions_mut().insert(Some(Identity {
            user_id: "agent".to_string(),
            tenant_id: tenant_id,
            role: "member".to_string(),
        }));
        return Ok(next.run(req).await);
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
        
        req.extensions_mut().insert(None::<Identity>);
        return Ok(next.run(req).await);
    };

    if !auth_header.starts_with("Bearer ") {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token = &auth_header[7..];

    // JWT validation logic
    let secret = std::env::var("ORIONIS_JWT_SECRET").unwrap_or_else(|_| "secret".to_string());
    
    // Check if it's an OIDC token from a known issuer
    if let Ok(header) = jsonwebtoken::decode_header(token) {
        if let Some(kid) = header.kid {
            // In a real OIDC setup, we'd fetch JWKS for this kid from the OIDC provider's discovery endpoint.
            // For now, we provide a hook for OIDC validation.
            if let Ok(identity) = validate_oidc_token(token, &kid).await {
                req.extensions_mut().insert(Some(identity));
                return Ok(next.run(req).await);
            }
        }
    }

    match decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    ) {
        Ok(token_data) => {
            let identity = Identity {
                user_id: token_data.claims.sub,
                tenant_id: token_data.claims.tenant_id,
                role: token_data.claims.role,
            };
            req.extensions_mut().insert(Some(identity));
            Ok(next.run(req).await)
        }
        Err(_) => Err(StatusCode::UNAUTHORIZED),
    }
}

async fn validate_oidc_token(_token: &str, _kid: &str) -> Result<Identity, StatusCode> {
    // Placeholder for real OIDC validation against providers like Google/Github/Auth0
    // This would typically use something like the `openidconnect` crate.
    let oidc_enabled = std::env::var("ORIONIS_OIDC_ENABLED").is_ok();
    if !oidc_enabled {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Mock successful OIDC validation
    Ok(Identity {
        user_id: "oidc-user".into(),
        tenant_id: "oidc-tenant".into(),
        role: "member".into(),
    })
}


pub fn get_identity(req: &axum::extract::Request) -> Option<Identity> {
    req.extensions().get::<Option<Identity>>().cloned().flatten()
}
