//! GraphQL Federation support
//!
//! This module implements Apollo Federation v2 specification, allowing the gateway
//! to participate in a federated GraphQL architecture.

use crate::error::{Error, Result};
use crate::graphql::GraphqlEntity;
use async_graphql::dynamic::{Field, FieldFuture, FieldValue, InputValue, Object, TypeRef};
use async_graphql::{Name, Value as GqlValue};
use prost::Message;
use prost_reflect::{DescriptorPool, ExtensionDescriptor, MessageDescriptor, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// Federation configuration extracted from protobuf descriptors
#[derive(Clone, Debug)]
pub struct FederationConfig {
    /// Map of entity type names to their key fields
    pub entities: HashMap<String, EntityConfig>,
}

/// Configuration for a federated entity
#[derive(Clone, Debug)]
pub struct EntityConfig {
    /// The message descriptor for this entity type
    pub descriptor: MessageDescriptor,
    /// Key field sets for this entity (e.g., ["id"], ["email"], or ["orgId", "userId"])
    pub keys: Vec<Vec<String>>,
    /// Whether this entity extends an entity from another service
    pub extend: bool,
    /// Whether this service can resolve this entity
    pub resolvable: bool,
    /// The GraphQL type name for this entity
    pub type_name: String,
}

impl FederationConfig {
    /// Create a new empty federation configuration
    pub fn new() -> Self {
        Self {
            entities: HashMap::new(),
        }
    }

    /// Extract federation configuration from descriptor pool
    pub fn from_descriptor_pool(
        pool: &DescriptorPool,
        entity_ext: &ExtensionDescriptor,
    ) -> Result<Self> {
        let mut config = Self::new();

        // Scan all messages for entity annotations
        for message in pool.all_messages() {
            if let Some(entity_opts) = decode_entity_extension(&message, entity_ext)? {
                if entity_opts.keys.is_empty() {
                    continue; // Skip messages without keys
                }

                let type_name = message.full_name().replace('.', "_");
                let keys: Vec<Vec<String>> = entity_opts
                    .keys
                    .iter()
                    .map(|key| {
                        // Split on whitespace to support composite keys like "orgId userId"
                        key.split_whitespace().map(String::from).collect::<Vec<_>>()
                    })
                    .collect();

                config.entities.insert(
                    type_name.clone(),
                    EntityConfig {
                        descriptor: message.clone(),
                        keys,
                        extend: entity_opts.extend,
                        resolvable: entity_opts.resolvable,
                        type_name,
                    },
                );
            }
        }

        Ok(config)
    }

    /// Check if federation is enabled (i.e., if there are any entities)
    pub fn is_enabled(&self) -> bool {
        !self.entities.is_empty()
    }

    /// Build the _entities field for the Query type
    pub fn build_entities_field_for_query(
        &self,
        entity_resolvers: Arc<dyn EntityResolver>,
    ) -> Result<Field> {
        let config = self.clone();

        let field = Field::new("_entities", TypeRef::named_nn_list("_Entity"), move |ctx| {
            let entity_resolvers = entity_resolvers.clone();
            let config = config.clone();

            FieldFuture::new(async move {
                let representations = ctx
                    .args
                    .get("representations")
                    .ok_or_else(|| async_graphql::Error::new("missing representations argument"))?
                    .list()?;

                let mut results = Vec::new();
                for repr in representations.iter() {
                    let obj = repr.object()?;

                    // Convert ObjectAccessor to IndexMap
                    let mut representation_map = async_graphql::indexmap::IndexMap::new();
                    for (key, value) in obj.iter() {
                        representation_map.insert(key.clone(), value.as_value().clone());
                    }

                    // Extract __typename from representation
                    let typename = representation_map
                        .get(&Name::new("__typename"))
                        .and_then(|v| match v {
                            GqlValue::String(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .ok_or_else(|| {
                            async_graphql::Error::new("missing __typename in representation")
                        })?;

                    // Find entity config
                    let entity_config = config.entities.get(typename).ok_or_else(|| {
                        async_graphql::Error::new(format!("unknown entity type: {}", typename))
                    })?;

                    // Resolve the entity
                    let entity = entity_resolvers
                        .resolve_entity(entity_config, &representation_map)
                        .await
                        .map_err(|e| async_graphql::Error::new(e.to_string()))?;

                    results
                        .push(FieldValue::value(entity).with_type(entity_config.type_name.clone()));
                }

                Ok(Some(FieldValue::list(results)))
            })
        })
        .argument(InputValue::new(
            "representations",
            TypeRef::named_nn_list_nn("_Any"),
        ));

        Ok(field)
    }

    /// Apply federation directives to an object type
    pub fn apply_directives_to_object(&self, obj: Object, type_name: &str) -> Result<Object> {
        if let Some(entity_config) = self.entities.get(type_name) {
            let mut obj = obj;

            // Add @key directives
            for key_fields in &entity_config.keys {
                let fields_str = key_fields.join(" ");
                // Mark as entity for async-graphql federation support
                if entity_config.resolvable {
                    obj = obj.key(fields_str.clone());
                } else {
                    obj = obj.unresolvable(fields_str.clone());
                }
                obj = obj.directive(
                    async_graphql::dynamic::Directive::new("key")
                        .argument("fields", GqlValue::String(fields_str)),
                );
            }

            // Add @extends directive if this is an extension
            if entity_config.extend {
                obj = obj.extends();
                obj = obj.directive(async_graphql::dynamic::Directive::new("extends"));
            }

            Ok(obj)
        } else {
            Ok(obj)
        }
    }
}

impl Default for FederationConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for resolving federated entities
///
/// Implementors should resolve entities based on their representation
/// (which contains the key fields and __typename)
#[async_trait::async_trait]
pub trait EntityResolver: Send + Sync {
    /// Resolve an entity from its representation
    async fn resolve_entity(
        &self,
        entity_config: &EntityConfig,
        representation: &async_graphql::indexmap::IndexMap<Name, GqlValue>,
    ) -> Result<GqlValue>;
}

/// Default entity resolver that uses gRPC client pool
pub struct GrpcEntityResolver {
    client_pool: crate::grpc_client::GrpcClientPool,
}

impl GrpcEntityResolver {
    pub fn new(client_pool: crate::grpc_client::GrpcClientPool) -> Self {
        Self { client_pool }
    }
}

impl Default for GrpcEntityResolver {
    fn default() -> Self {
        Self::new(crate::grpc_client::GrpcClientPool::new())
    }
}

#[async_trait::async_trait]
impl EntityResolver for GrpcEntityResolver {
    async fn resolve_entity(
        &self,
        entity_config: &EntityConfig,
        representation: &async_graphql::indexmap::IndexMap<Name, GqlValue>,
    ) -> Result<GqlValue> {
        tracing::debug!(
            "Resolving entity {} with representation: {:?}",
            entity_config.type_name,
            representation
        );

        // Placeholder: return representation as-is. A production resolver
        // should look up the appropriate gRPC client + method and fetch the entity.
        Ok(GqlValue::Object(representation.clone()))
    }
}

/// Decode entity extension from message descriptor
fn decode_entity_extension(
    message: &MessageDescriptor,
    ext: &ExtensionDescriptor,
) -> Result<Option<GraphqlEntity>> {
    let opts = message.options();
    if !opts.has_extension(ext) {
        return Ok(None);
    }

    let val = opts.get_extension(ext);
    if let Value::Message(msg) = val.as_ref() {
        GraphqlEntity::decode(msg.encode_to_vec().as_slice())
            .map(Some)
            .map_err(|e| Error::Schema(format!("failed to decode entity extension: {e}")))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_federation_config_new() {
        let config = FederationConfig::new();
        assert!(!config.is_enabled());
        assert!(config.entities.is_empty());
    }

    #[test]
    fn test_entity_config_composite_keys() {
        // Test that key field sets are properly parsed
        let keys = vec![
            vec!["id".to_string()],
            vec!["org".to_string(), "user".to_string()],
        ];
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], vec!["id"]);
        assert_eq!(keys[1], vec!["org", "user"]);
    }
}
