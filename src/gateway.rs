//! Gateway builder and main orchestration

use crate::error::{GraphQLError, Result};
use crate::grpc_client::{GrpcClient, GrpcClientPool};
use crate::middleware::Middleware;
use crate::runtime::ServeMux;
use crate::schema::{DynamicSchema, SchemaBuilder};
use axum::Router;
use std::path::Path;
use std::sync::Arc;

/// Main Gateway struct - entry point for the library
pub struct Gateway {
    mux: ServeMux,
    client_pool: GrpcClientPool,
    schema: DynamicSchema,
}

impl Gateway {
    /// Create a new gateway builder
    pub fn builder() -> GatewayBuilder {
        GatewayBuilder::new()
    }

    /// Get the ServeMux
    pub fn mux(&self) -> &ServeMux {
        &self.mux
    }

    /// Access the built GraphQL schema
    pub fn schema(&self) -> &DynamicSchema {
        &self.schema
    }

    /// Get the client pool
    pub fn client_pool(&self) -> &GrpcClientPool {
        &self.client_pool
    }

    /// Convert gateway into Axum router
    pub fn into_router(self) -> Router {
        self.mux.into_router()
    }
}

/// Builder for creating a Gateway
pub struct GatewayBuilder {
    client_pool: GrpcClientPool,
    schema_builder: SchemaBuilder,
    middlewares: Vec<Arc<dyn Middleware>>,
    error_handler: Option<Arc<dyn Fn(Vec<GraphQLError>) + Send + Sync>>,
    entity_resolver: Option<Arc<dyn crate::federation::EntityResolver>>,
    service_allowlist: Option<std::collections::HashSet<String>>,
}

impl GatewayBuilder {
    /// Create a new gateway builder
    pub fn new() -> Self {
        Self {
            client_pool: GrpcClientPool::new(),
            schema_builder: SchemaBuilder::new(),
            middlewares: Vec::new(),
            error_handler: None,
            entity_resolver: None,
            service_allowlist: None,
        }
    }

    /// Add a gRPC client to the pool
    pub fn add_grpc_client(self, name: impl Into<String>, client: GrpcClient) -> Self {
        self.client_pool.add(name, client);
        self
    }

    /// Add many gRPC clients in one shot.
    pub fn add_grpc_clients<I>(self, clients: I) -> Self
    where
        I: IntoIterator<Item = (String, GrpcClient)>,
    {
        for (name, client) in clients {
            self.client_pool.add(name, client);
        }
        self
    }

    /// Add middleware
    pub fn add_middleware<M: Middleware + 'static>(mut self, middleware: M) -> Self {
        self.middlewares.push(Arc::new(middleware));
        self
    }

    /// Provide a protobuf descriptor set (bytes)
    pub fn with_descriptor_set_bytes(mut self, bytes: impl AsRef<[u8]>) -> Self {
        self.schema_builder = self.schema_builder.with_descriptor_set_bytes(bytes);
        self
    }

    /// Provide a custom entity resolver for federation.
    pub fn with_entity_resolver(
        mut self,
        resolver: Arc<dyn crate::federation::EntityResolver>,
    ) -> Self {
        self.entity_resolver = Some(resolver.clone());
        self.schema_builder = self.schema_builder.with_entity_resolver(resolver);
        self
    }

    /// Restrict the schema to the provided gRPC service full names.
    pub fn with_services<I, S>(mut self, services: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let set: std::collections::HashSet<String> = services.into_iter().map(Into::into).collect();
        self.schema_builder = self.schema_builder.with_services(set.clone());
        self.service_allowlist = Some(set);
        self
    }

    /// Enable GraphQL federation features.
    pub fn enable_federation(mut self) -> Self {
        self.schema_builder = self.schema_builder.enable_federation();
        self
    }

    /// Provide a protobuf descriptor set file
    pub fn with_descriptor_set_file(mut self, path: impl AsRef<Path>) -> Result<Self> {
        self.schema_builder = self.schema_builder.with_descriptor_set_file(path)?;
        Ok(self)
    }

    /// Provide a handler to inspect/augment GraphQL errors before they are returned.
    pub fn with_error_handler<F>(mut self, handler: F) -> Self
    where
        F: Fn(Vec<GraphQLError>) + Send + Sync + 'static,
    {
        self.error_handler = Some(Arc::new(handler));
        self
    }

    /// Build the gateway
    pub fn build(self) -> Result<Gateway> {
        let mut schema_builder = self.schema_builder;
        if let Some(resolver) = self.entity_resolver {
            schema_builder = schema_builder.with_entity_resolver(resolver);
        }
        if let Some(services) = self.service_allowlist {
            schema_builder = schema_builder.with_services(services);
        }

        let schema = schema_builder.build(&self.client_pool)?;
        let mut mux = ServeMux::new(schema.clone());

        // Add middlewares
        for middleware in self.middlewares {
            mux.add_middleware(middleware);
        }

        if let Some(handler) = self.error_handler {
            mux.set_error_handler_arc(handler);
        }

        Ok(Gateway {
            mux,
            client_pool: self.client_pool,
            schema,
        })
    }

    /// Build and start the gateway server
    pub async fn serve(self, addr: impl Into<String>) -> Result<()> {
        let gateway = self.build()?;
        let addr = addr.into();
        let listener = tokio::net::TcpListener::bind(&addr).await?;

        tracing::info!("Gateway server listening on {}", addr);

        let app = gateway.into_router();
        axum::serve(listener, app).await?;

        Ok(())
    }
}

impl Default for GatewayBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_creation() {
        let builder = GatewayBuilder::new();
        let result = builder.build();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_grpc_client_pool() {
        let pool = GrpcClientPool::new();

        // In a real test, you'd create actual clients
        // For now, just test the pool structure
        assert_eq!(pool.names().len(), 0);
    }
}
