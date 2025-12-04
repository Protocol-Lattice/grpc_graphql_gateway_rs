//! Type definitions for GraphQL-gRPC gateway

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// GraphQL request from client
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GraphQLRequest {
    /// GraphQL query string
    #[serde(default)]
    pub query: String,

    /// Operation name (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,

    /// Variables for the query
    #[serde(default)]
    pub variables: HashMap<String, serde_json::Value>,
}

/// GraphQL response to client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLResponse {
    /// Response data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,

    /// Errors if any
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub errors: Vec<crate::error::GraphQLError>,
}

impl GraphQLResponse {
    /// Create a successful response
    pub fn success(data: serde_json::Value) -> Self {
        Self {
            data: Some(data),
            errors: Vec::new(),
        }
    }

    /// Create an error response
    pub fn error(error: crate::error::GraphQLError) -> Self {
        Self {
            data: None,
            errors: vec![error],
        }
    }

    /// Create an error response from multiple errors
    pub fn errors(errors: Vec<crate::error::GraphQLError>) -> Self {
        Self { data: None, errors }
    }
}

/// GraphQL schema type (Query, Mutation, or Subscription)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SchemaType {
    Query,
    Mutation,
    Subscription,
}

/// Configuration for a GraphQL field from gRPC method
///
/// This struct holds the metadata extracted from the protobuf options
/// (`graphql.schema`) for a specific gRPC method.
#[derive(Debug, Clone)]
pub struct FieldConfig {
    /// Field name in GraphQL schema
    pub name: String,

    /// Schema type (Query/Mutation/Subscription)
    pub schema_type: SchemaType,

    /// Whether the field is required
    pub required: bool,

    /// gRPC service name
    pub service_name: String,

    /// gRPC method name
    pub method_name: String,

    /// Whether this is a streaming method
    pub streaming: bool,
}

/// gRPC service configuration
///
/// Represents a configured gRPC service that the gateway connects to.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    /// Service name
    pub name: String,

    /// gRPC host:port
    pub endpoint: String,

    /// Whether to use insecure connection
    pub insecure: bool,
}
