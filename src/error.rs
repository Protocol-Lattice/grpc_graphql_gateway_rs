//! Error types for the gRPC-GraphQL gateway

use thiserror::Error;

/// Result type alias using our Error type
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for the gateway
///
/// This enum covers all possible errors that can occur within the gateway,
/// including gRPC errors, schema errors, and runtime errors.
#[derive(Error, Debug)]
pub enum Error {
    /// gRPC transport errors
    #[error("gRPC error: {0}")]
    Grpc(#[from] tonic::Status),

    /// gRPC transport errors
    #[error("gRPC transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    /// GraphQL schema errors
    #[error("GraphQL schema error: {0}")]
    Schema(String),

    /// Invalid request errors
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Authentication/authorization errors
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    /// Middleware errors
    #[error("Middleware error: {0}")]
    Middleware(String),

    /// Serialization errors
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Connection errors
    #[error("Connection error: {0}")]
    Connection(String),

    /// WebSocket errors
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    /// Internal errors
    #[error("Internal error: {0}")]
    Internal(String),

    /// IO errors
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Any other error
    #[error("Error: {0}")]
    Other(#[from] anyhow::Error),
}

impl Error {
    /// Convert error to GraphQL error format
    pub fn to_graphql_error(&self) -> GraphQLError {
        GraphQLError {
            message: self.to_string(),
            extensions: self.extensions(),
        }
    }

    /// Get error code for extensions
    fn extensions(&self) -> std::collections::HashMap<String, serde_json::Value> {
        let mut map = std::collections::HashMap::new();
        let code = match self {
            Error::Grpc(_) => "GRPC_ERROR",
            Error::Transport(_) => "TRANSPORT_ERROR",
            Error::Schema(_) => "SCHEMA_ERROR",
            Error::InvalidRequest(_) => "INVALID_REQUEST",
            Error::Unauthorized(_) => "UNAUTHORIZED",
            Error::Middleware(_) => "MIDDLEWARE_ERROR",
            Error::Serialization(_) => "SERIALIZATION_ERROR",
            Error::Connection(_) => "CONNECTION_ERROR",
            Error::WebSocket(_) => "WEBSOCKET_ERROR",
            Error::Internal(_) => "INTERNAL_ERROR",
            Error::Io(_) => "IO_ERROR",
            Error::Other(_) => "UNKNOWN_ERROR",
        };
        map.insert("code".to_string(), serde_json::json!(code));
        map
    }
}

/// GraphQL error response format
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GraphQLError {
    pub message: String,
    #[serde(skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub extensions: std::collections::HashMap<String, serde_json::Value>,
}

impl From<Error> for GraphQLError {
    fn from(err: Error) -> Self {
        err.to_graphql_error()
    }
}
