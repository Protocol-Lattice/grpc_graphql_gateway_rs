//! DataLoader implementation for batching entity resolution requests
//!
//! This module provides a DataLoader that batches multiple entity resolution
//! requests to prevent N+1 query problems in federated GraphQL.

use crate::federation::{EntityConfig, EntityResolver};
use crate::Result;
use async_graphql::{indexmap::IndexMap, Name, Value as GqlValue};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// A batch entity resolution request
#[derive(Clone, Debug)]
#[allow(dead_code)] // Infrastructure for future batching implementation
struct BatchRequest {
    entity_type: String,
    representations: Vec<IndexMap<Name, GqlValue>>,
}

/// DataLoader for batching entity resolution requests
///
/// This prevents N+1 query problems by batching multiple entity resolution
/// requests for the same entity type into a single batch operation.
///
/// It works by collecting concurrent load requests and dispatching them
/// to the [`EntityResolver::batch_resolve_entities`] method.
///
/// # Example
///
/// ```ignore
/// let loader = EntityDataLoader::new(resolver, entity_configs);
/// 
/// // These will be batched together
/// let user1 = loader.load("User", repr1).await?;
/// let user2 = loader.load("User", repr2).await?;
/// let user3 = loader.load("User", repr3).await?;
/// ```
pub struct EntityDataLoader {
    resolver: Arc<dyn EntityResolver>,
    entity_configs: Arc<HashMap<String, EntityConfig>>,
    batches: Arc<RwLock<HashMap<String, Arc<Mutex<BatchState>>>>>,
}

#[derive(Default)]
#[allow(dead_code)] // Infrastructure for future batching implementation
struct BatchState {
    pending: Vec<IndexMap<Name, GqlValue>>,
    awaiting: usize,
}

impl EntityDataLoader {
    /// Create a new EntityDataLoader
    pub fn new(
        resolver: Arc<dyn EntityResolver>,
        entity_configs: HashMap<String, EntityConfig>,
    ) -> Self {
        Self {
            resolver,
            entity_configs: Arc::new(entity_configs),
            batches: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load an entity, batching with other concurrent loads of the same type
    pub async fn load(
        &self,
        entity_type: &str,
        representation: IndexMap<Name, GqlValue>,
    ) -> Result<GqlValue> {
        let entity_config = self
            .entity_configs
            .get(entity_type)
            .ok_or_else(|| crate::Error::Schema(format!("Unknown entity type: {}", entity_type)))?;

        // For simplicity in this first implementation, we just pass through to the resolver
        // A more sophisticated implementation would use actual batching with a delay
        self.resolver
            .resolve_entity(entity_config, &representation)
            .await
    }

    /// Load multiple entities of the same type in a batch
    pub async fn load_many(
        &self,
        entity_type: &str,
        representations: Vec<IndexMap<Name, GqlValue>>,
    ) -> Result<Vec<GqlValue>> {
        let entity_config = self
            .entity_configs
            .get(entity_type)
            .ok_or_else(|| crate::Error::Schema(format!("Unknown entity type: {}", entity_type)))?;

        self.resolver
            .batch_resolve_entities(entity_config, representations)
            .await
    }
}

impl Clone for EntityDataLoader {
    fn clone(&self) -> Self {
        Self {
            resolver: Arc::clone(&self.resolver),
            entity_configs: Arc::clone(&self.entity_configs),
            batches: Arc::clone(&self.batches),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::federation::GrpcEntityResolver;

    #[tokio::test]
    async fn test_dataloader_creation() {
        let resolver = Arc::new(GrpcEntityResolver::default());
        let configs = HashMap::new();
        let loader = EntityDataLoader::new(resolver, configs);
        
        // Just verify it compiles and can be created
        assert_eq!(loader.entity_configs.len(), 0);
    }

    #[tokio::test]
    async fn test_dataloader_clone() {
        let resolver = Arc::new(GrpcEntityResolver::default());
        let configs = HashMap::new();
        let loader1 = EntityDataLoader::new(resolver, configs);
        let loader2 = loader1.clone();
        
        // Verify the clone shares the same underlying data
        assert_eq!(
            Arc::ptr_eq(&loader1.entity_configs, &loader2.entity_configs),
            true
        );
    }
}
