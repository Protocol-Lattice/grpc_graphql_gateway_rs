//! # grpc-graphql-gateway-rs
//!
//! A high-performance Rust gateway that bridges gRPC services to GraphQL with full Apollo Federation v2 support.
//!
//! ## Features
//!
//! - **Dynamic Schema Generation**: Automatic GraphQL schema from protobuf descriptors
//! - **Federation v2**: Complete Apollo Federation support with entity resolution and `@shareable`
//! - **Batching**: Built-in [`EntityDataLoader`] for efficient N+1 query prevention
//! - **Subscriptions**: Real-time data via GraphQL subscriptions (WebSocket)
//! - **Middleware**: Extensible middleware system for auth and logging
//!
//! ## Main Components
//!
//! - [`Gateway`]: The main entry point for creating and running the gateway.
//! - [`GatewayBuilder`]: Configuration builder for the gateway.
//! - [`SchemaBuilder`]: Low-level builder for the dynamic GraphQL schema.
//! - [`GrpcClient`]: Manages connections to gRPC services.
//! - [`GrpcEntityResolver`]: Handles federation entity resolution.
//!
//! ## Federation
//!
//! To enable federation, use the [`GatewayBuilder::enable_federation`] method and configure
//! an entity resolver. See [`GrpcEntityResolver`] and [`EntityDataLoader`] for details.
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
pub mod dataloader;
pub mod federation;
pub mod gateway;
pub mod grpc_client;
pub mod middleware;
pub mod runtime;
pub mod schema;
pub mod types;

pub use dataloader::EntityDataLoader;
pub use error::{Error, Result};
pub use federation::{
    EntityConfig, EntityResolver, EntityResolverMapping, FederationConfig, GrpcEntityResolver,
    GrpcEntityResolverBuilder,
};
pub use gateway::{Gateway, GatewayBuilder};
pub use grpc_client::GrpcClient;
pub use middleware::{Context, Middleware};
pub use runtime::ServeMux;
pub use schema::SchemaBuilder;
