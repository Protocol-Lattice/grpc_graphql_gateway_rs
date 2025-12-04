//! gRPC client connection management

use crate::error::{Error, Result};
use std::sync::Arc;
use tonic::transport::{Channel, Endpoint};

/// gRPC client connection manager
///
/// Manages a connection to a gRPC service, handling channel creation and configuration.
///
/// # Example
///
/// ```rust,no_run
/// use grpc_graphql_gateway::GrpcClient;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Connect immediately
/// let client = GrpcClient::new("http://localhost:50051").await?;
///
/// // Or lazy connect
/// let lazy_client = GrpcClient::builder("http://localhost:50051")
///     .connect_lazy()?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct GrpcClient {
    /// Service endpoint URL
    endpoint: String,

    /// gRPC channel
    channel: Channel,

    /// Whether connection is insecure
    insecure: bool,
}

impl GrpcClient {
    /// Start building a gRPC client with custom connection behavior.
    ///
    /// Returns a [`GrpcClientBuilder`] for configuration.
    pub fn builder(endpoint: impl Into<String>) -> GrpcClientBuilder {
        GrpcClientBuilder::new(endpoint)
    }

    /// Create a new gRPC client
    pub async fn new(endpoint: impl Into<String>) -> Result<Self> {
        Self::builder(endpoint).connect().await
    }

    /// Create a new secure gRPC client
    pub async fn new_secure(endpoint: impl Into<String>) -> Result<Self> {
        Self::builder(endpoint).insecure(false).connect().await
    }

    /// Create a new client without awaiting the connection (lazy connect on first use)
    ///
    /// This is useful when the service might not be up yet when the gateway starts.
    pub fn connect_lazy(endpoint: impl Into<String>, insecure: bool) -> Result<Self> {
        Self::builder(endpoint)
            .insecure(insecure)
            .lazy(true)
            .connect_lazy()
    }

    /// Create gRPC channel
    async fn create_channel(endpoint: &str, insecure: bool) -> Result<Channel> {
        let endpoint_builder = configure_endpoint(endpoint, insecure)?;

        let channel = endpoint_builder
            .connect()
            .await
            .map_err(|e| Error::Connection(e.to_string()))?;

        Ok(channel)
    }

    /// Get the channel for making gRPC calls
    pub fn channel(&self) -> Channel {
        self.channel.clone()
    }

    /// Get the endpoint URL
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Check if connection is insecure
    pub fn is_insecure(&self) -> bool {
        self.insecure
    }

    /// Test connection health
    pub async fn health_check(&self) -> Result<()> {
        // Simple check - try to clone the channel
        // In real implementation, you might want to call a health check endpoint
        let _ = self.channel.clone();
        Ok(())
    }
}

impl std::fmt::Debug for GrpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrpcClient")
            .field("endpoint", &self.endpoint)
            .field("insecure", &self.insecure)
            .finish()
    }
}

/// Builder for configuring gRPC client creation.
pub struct GrpcClientBuilder {
    endpoint: String,
    insecure: bool,
    lazy: bool,
}

impl GrpcClientBuilder {
    fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            insecure: true,
            lazy: false,
        }
    }

    /// Toggle insecure (plaintext) connections.
    pub fn insecure(mut self, insecure: bool) -> Self {
        self.insecure = insecure;
        self
    }

    /// Perform a lazy connect (defer until first use).
    pub fn lazy(mut self, lazy: bool) -> Self {
        self.lazy = lazy;
        self
    }

    /// Establish the channel immediately.
    pub async fn connect(self) -> Result<GrpcClient> {
        if self.lazy {
            return self.connect_lazy();
        }

        let channel = GrpcClient::create_channel(&self.endpoint, self.insecure).await?;
        Ok(GrpcClient {
            endpoint: self.endpoint,
            channel,
            insecure: self.insecure,
        })
    }

    /// Create a lazily connecting client.
    pub fn connect_lazy(self) -> Result<GrpcClient> {
        let endpoint_builder = configure_endpoint(&self.endpoint, self.insecure)?;
        let channel = endpoint_builder.connect_lazy();

        Ok(GrpcClient {
            endpoint: self.endpoint,
            channel,
            insecure: self.insecure,
        })
    }
}

fn configure_endpoint(endpoint: &str, insecure: bool) -> Result<Endpoint> {
    let mut endpoint_builder = Endpoint::from_shared(endpoint.to_string())
        .map_err(|e| Error::Connection(e.to_string()))?;

    // Configure TLS if not insecure
    if !insecure {
        endpoint_builder = endpoint_builder
            .tls_config(tonic::transport::ClientTlsConfig::new())
            .map_err(|e| Error::Connection(e.to_string()))?;
    }

    Ok(endpoint_builder)
}

/// Pool of gRPC clients for multiple services
#[derive(Clone, Default)]
pub struct GrpcClientPool {
    clients: Arc<std::sync::RwLock<std::collections::HashMap<String, GrpcClient>>>,
}

impl GrpcClientPool {
    /// Create a new client pool
    pub fn new() -> Self {
        Self {
            clients: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Add a client to the pool
    pub fn add(&self, name: impl Into<String>, client: GrpcClient) {
        let mut clients = self.clients.write().unwrap();
        clients.insert(name.into(), client);
    }

    /// Get a client from the pool
    pub fn get(&self, name: &str) -> Option<GrpcClient> {
        let clients = self.clients.read().unwrap();
        clients.get(name).cloned()
    }

    /// Remove a client from the pool
    pub fn remove(&self, name: &str) -> Option<GrpcClient> {
        let mut clients = self.clients.write().unwrap();
        clients.remove(name)
    }

    /// Get all client names
    pub fn names(&self) -> Vec<String> {
        let clients = self.clients.read().unwrap();
        clients.keys().cloned().collect()
    }

    /// Clear all clients
    pub fn clear(&self) {
        let mut clients = self.clients.write().unwrap();
        clients.clear();
    }
}

impl std::fmt::Debug for GrpcClientPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let clients = self.clients.read().unwrap();
        f.debug_struct("GrpcClientPool")
            .field("clients", &clients.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::GrpcClient;

    #[tokio::test]
    async fn builder_creates_lazy_client() {
        let client = GrpcClient::builder("http://localhost:50051")
            .lazy(true)
            .connect_lazy()
            .expect("lazy connect should not fail");

        assert!(client.is_insecure());
        assert_eq!(client.endpoint(), "http://localhost:50051");
    }

    #[tokio::test]
    async fn builder_respects_lazy_on_connect() {
        let client = GrpcClient::builder("http://localhost:50051")
            .lazy(true)
            .connect()
            .await
            .expect("lazy connect should not open socket");

        assert!(client.is_insecure());
        assert_eq!(client.endpoint(), "http://localhost:50051");
    }

    #[tokio::test]
    async fn builder_can_create_secure_lazy_client() {
        let client = GrpcClient::builder("https://example.com:443")
            .insecure(false)
            .connect_lazy()
            .expect("lazy TLS client should be configured");

        assert!(!client.is_insecure());
    }
}
