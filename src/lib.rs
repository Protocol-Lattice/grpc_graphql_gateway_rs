//! # grpc-graphql-gateway-rs
//!
//! A Rust implementation of gRPC-GraphQL gateway that generates GraphQL execution
//! code from gRPC services, inspired by the Go version.
//!
//! ## Features
//!
//! - Convert gRPC services to GraphQL queries, mutations, and subscriptions
//! - Support for streaming gRPC responses via GraphQL subscriptions
//! - Middleware support for authentication and custom logic
//! - WebSocket support for GraphQL subscriptions
//!
//! ## Example
//!
//! ```rust,no_run
//! use grpc_graphql_gateway::{Gateway, GrpcClient};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let grpc_client = GrpcClient::new("http://localhost:50051").await?;
//!     
//!     let gateway = Gateway::builder()
//!         .add_grpc_client("greeter", grpc_client)
//!         .build()?;
//!     
//!     let app = gateway.into_router();
//!     
//!     let listener = tokio::net::TcpListener::bind("0.0.0.0:8888").await?;
//!     axum::serve(listener, app).await?;
//!     
//!     Ok(())
//! }
//! ```

/// Generated types for graphql.proto options.
#[allow(clippy::all)]
pub mod graphql {
    include!("generated/graphql.rs");
}

pub mod error;
pub mod federation;
pub mod gateway;
pub mod grpc_client;
pub mod middleware;
pub mod runtime;
pub mod schema;
pub mod types;

pub use error::{Error, Result};
pub use federation::{EntityResolver, FederationConfig, GrpcEntityResolver};
pub use gateway::{Gateway, GatewayBuilder};
pub use grpc_client::GrpcClient;
pub use middleware::{Context, Middleware};
pub use runtime::ServeMux;
pub use schema::SchemaBuilder;
