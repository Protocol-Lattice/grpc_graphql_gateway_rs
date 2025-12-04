//! Middleware support for the gateway

use crate::error::Result;
use axum::http::Request;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Context passed to middleware
#[derive(Debug)]
pub struct Context {
    /// Request headers and metadata
    pub headers: axum::http::HeaderMap,

    /// Additional context data
    pub extensions: std::collections::HashMap<String, serde_json::Value>,
}

impl Context {
    /// Create a new context from request
    pub fn from_request<B>(req: &Request<B>) -> Self {
        Self {
            headers: req.headers().clone(),
            extensions: std::collections::HashMap::new(),
        }
    }

    /// Insert extension data
    pub fn insert(&mut self, key: String, value: serde_json::Value) {
        self.extensions.insert(key, value);
    }

    /// Get extension data
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.extensions.get(key)
    }
}

/// Middleware trait for processing requests
///
/// Middleware can intercept requests before they are processed by the GraphQL engine.
///
/// # Example
///
/// ```rust
/// use grpc_graphql_gateway::middleware::{Middleware, Context};
/// use grpc_graphql_gateway::Result;
///
/// struct MyMiddleware;
///
/// #[async_trait::async_trait]
/// impl Middleware for MyMiddleware {
///     async fn call(&self, ctx: &mut Context) -> Result<()> {
///         println!("Processing request");
///         Ok(())
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait Middleware: Send + Sync {
    /// Process the request context
    async fn call(&self, ctx: &mut Context) -> Result<()>;
}

/// Type alias for boxed middleware
pub type BoxMiddleware = Box<dyn Middleware>;

/// Middleware function type
pub type MiddlewareFn =
    Arc<dyn Fn(&mut Context) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync>;

/// CORS middleware
///
/// Handles Cross-Origin Resource Sharing (CORS) headers.
/// Note: This is a placeholder implementation. In a real application,
/// you should use `tower_http::cors::CorsLayer` with Axum.
#[derive(Debug, Clone)]
pub struct CorsMiddleware {
    pub allow_origins: Vec<String>,
    pub allow_methods: Vec<String>,
    pub allow_headers: Vec<String>,
}

impl Default for CorsMiddleware {
    fn default() -> Self {
        Self {
            allow_origins: vec!["*".to_string()],
            allow_methods: vec!["GET".to_string(), "POST".to_string()],
            allow_headers: vec!["Content-Type".to_string(), "Authorization".to_string()],
        }
    }
}

#[async_trait::async_trait]
impl Middleware for CorsMiddleware {
    async fn call(&self, _ctx: &mut Context) -> Result<()> {
        // CORS is handled by tower-http, this is just a placeholder
        Ok(())
    }
}

/// Authentication middleware
///
/// Validates the `Authorization` header using a provided validation function.
/// If validation fails, it returns an `Unauthorized` error.
#[derive(Clone)]
pub struct AuthMiddleware {
    pub validate: Arc<dyn Fn(&str) -> bool + Send + Sync>,
}

#[async_trait::async_trait]
impl Middleware for AuthMiddleware {
    async fn call(&self, ctx: &mut Context) -> Result<()> {
        if let Some(auth_header) = ctx.headers.get("authorization") {
            if let Ok(token) = auth_header.to_str() {
                if (self.validate)(token) {
                    return Ok(());
                }
            }
        }
        Err(crate::error::Error::Unauthorized(
            "Invalid or missing authorization".to_string(),
        ))
    }
}

/// Logging middleware
///
/// Logs incoming GraphQL requests using the `tracing` crate.
#[derive(Debug, Clone, Default)]
pub struct LoggingMiddleware;

#[async_trait::async_trait]
impl Middleware for LoggingMiddleware {
    async fn call(&self, ctx: &mut Context) -> Result<()> {
        tracing::debug!("Processing GraphQL request with headers: {:?}", ctx.headers);
        Ok(())
    }
}
