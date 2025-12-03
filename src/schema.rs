//! GraphQL schema building from gRPC service descriptors.
//!
//! This module reads protobuf descriptors (including the custom options defined in
//! `proto/graphql.proto`) and builds an `async-graphql` dynamic schema whose
//! resolvers proxy calls to the appropriate gRPC methods via `tonic`.

use crate::error::{Error, Result};
use crate::federation::{EntityResolver, FederationConfig, GrpcEntityResolver};
use crate::graphql::{GraphqlField, GraphqlResponse, GraphqlSchema, GraphqlService, GraphqlType};
use crate::grpc_client::{GrpcClient, GrpcClientPool};
use async_graphql::dynamic::{
    Enum, EnumItem, Field, FieldFuture, FieldValue, InputObject, InputValue, Object,
    ResolverContext, Schema as AsyncSchema, Subscription, SubscriptionField,
    SubscriptionFieldFuture, TypeRef,
};
use async_graphql::futures_util::StreamExt;
use async_graphql::indexmap::IndexMap;
use async_graphql::{Name, UploadValue, Value as GqlValue};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use prost::bytes::Buf;
use prost::Message;
use prost_reflect::{
    DescriptorPool, DynamicMessage, EnumDescriptor, ExtensionDescriptor, FieldDescriptor, Kind,
    MapKey, MessageDescriptor, ReflectMessage, Value,
};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tonic::client::Grpc;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::codegen::http;
use tonic::Status;

/// Dynamic schema wrapper
#[derive(Clone)]
pub struct DynamicSchema {
    inner: AsyncSchema,
}

impl DynamicSchema {
    /// Execute a GraphQL request
    pub async fn execute(&self, request: async_graphql::Request) -> async_graphql::Response {
        self.inner.execute(request).await
    }

    /// Access the executor (used for HTTP/WS integration)
    pub fn executor(&self) -> AsyncSchema {
        self.inner.clone()
    }
}

/// Schema builder for GraphQL gateway
pub struct SchemaBuilder {
    descriptor_bytes: Option<Vec<u8>>,
    federation: bool,
    entity_resolver: Option<std::sync::Arc<dyn EntityResolver>>,
    service_allowlist: Option<HashSet<String>>,
}

impl SchemaBuilder {
    /// Create a new schema builder
    pub fn new() -> Self {
        Self {
            descriptor_bytes: None,
            federation: false,
            entity_resolver: None,
            service_allowlist: None,
        }
    }

    /// Provide a descriptor set from bytes
    pub fn with_descriptor_set_bytes(mut self, bytes: impl AsRef<[u8]>) -> Self {
        self.descriptor_bytes = Some(bytes.as_ref().to_vec());
        self
    }

    /// Provide a descriptor set from a file
    pub fn with_descriptor_set_file(mut self, path: impl AsRef<Path>) -> Result<Self> {
        let data = std::fs::read(path).map_err(Error::Io)?;
        self.descriptor_bytes = Some(data);
        Ok(self)
    }

    /// Enable GraphQL federation support (adds _service/_entities when types are annotated as entities).
    pub fn enable_federation(mut self) -> Self {
        self.federation = true;
        self
    }

    /// Override the entity resolver used for federation.
    pub fn with_entity_resolver(mut self, resolver: std::sync::Arc<dyn EntityResolver>) -> Self {
        self.entity_resolver = Some(resolver);
        self
    }

    /// Limit the schema to specific gRPC services (full protobuf service names).
    pub fn with_services<I, S>(mut self, services: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.service_allowlist = Some(services.into_iter().map(Into::into).collect());
        self
    }

    /// Build the GraphQL schema from the provided descriptor set.
    pub fn build(self, client_pool: &GrpcClientPool) -> Result<DynamicSchema> {
        let bytes = self
            .descriptor_bytes
            .ok_or_else(|| Error::Schema("descriptor set is required".into()))?;

        let pool = DescriptorPool::decode(bytes.as_slice())
            .map_err(|e| Error::Schema(format!("failed to decode descriptor set: {e}")))?;

        let method_ext = pool
            .get_extension_by_name("graphql.schema")
            .ok_or_else(|| Error::Schema("missing graphql.schema extension".into()))?;
        let service_ext = pool
            .get_extension_by_name("graphql.service")
            .ok_or_else(|| Error::Schema("missing graphql.service extension".into()))?;
        let field_ext = pool
            .get_extension_by_name("graphql.field")
            .ok_or_else(|| Error::Schema("missing graphql.field extension".into()))?;

        // Load entity extension if federation is enabled
        let entity_ext = if self.federation {
            pool.get_extension_by_name("graphql.entity")
        } else {
            None
        };

        // Extract federation configuration
        let federation_config = if let Some(entity_ext) = entity_ext.as_ref() {
            FederationConfig::from_descriptor_pool(&pool, entity_ext)?
        } else {
            FederationConfig::new()
        };

        let mut registry = TypeRegistry::default();

        let mut query_root: Option<Object> = None;
        let mut mutation_root: Option<Object> = None;
        let mut subscription_root: Option<Subscription> = None;

        for service in pool.services() {
            if let Some(allowlist) = self.service_allowlist.as_ref() {
                if !allowlist.contains(service.full_name()) {
                    continue;
                }
            }

            let service_options =
                decode_extension::<GraphqlService>(&service.options(), &service_ext)?
                    .unwrap_or_default();

            if !service_options.host.is_empty() && client_pool.get(service.full_name()).is_none() {
                let client = GrpcClient::connect_lazy(
                    service_options.host.clone(),
                    service_options.insecure,
                )?;
                client_pool.add(service.full_name(), client);
            }

            for method in service.methods() {
                let Some(schema_opts) =
                    decode_extension::<GraphqlSchema>(&method.options(), &method_ext)?
                else {
                    continue;
                };

                let graphql_type =
                    GraphqlType::try_from(schema_opts.r#type).unwrap_or(GraphqlType::Query);
                let field_name = schema_opts.name.clone();
                if field_name.is_empty() {
                    continue;
                }

                match graphql_type {
                    GraphqlType::Query | GraphqlType::Resolver => {
                        let field = build_field(
                            field_name,
                            &service,
                            &method,
                            &schema_opts,
                            field_ext.clone(),
                            &mut registry,
                            client_pool.clone(),
                        )?;
                        let mut query = query_root.take().unwrap_or_else(|| Object::new("Query"));
                        query = query.field(field);
                        query_root = Some(query);
                    }
                    GraphqlType::Mutation => {
                        let field = build_field(
                            field_name,
                            &service,
                            &method,
                            &schema_opts,
                            field_ext.clone(),
                            &mut registry,
                            client_pool.clone(),
                        )?;
                        let mut mutation = mutation_root
                            .take()
                            .unwrap_or_else(|| Object::new("Mutation"));
                        mutation = mutation.field(field);
                        mutation_root = Some(mutation);
                    }
                    GraphqlType::Subscription => {
                        let field = build_subscription_field(
                            field_name,
                            &service,
                            &method,
                            &schema_opts,
                            field_ext.clone(),
                            &mut registry,
                            client_pool.clone(),
                        )?;
                        let mut subscription = subscription_root
                            .take()
                            .unwrap_or_else(|| Subscription::new("Subscription"));
                        subscription = subscription.field(field);
                        subscription_root = Some(subscription);
                    }
                }
            }
        }

        let query_root = query_root.unwrap_or_else(placeholder_query_root);

        let mut schema_builder = AsyncSchema::build(
            query_root.type_name(),
            mutation_root.as_ref().map(Object::type_name),
            subscription_root.as_ref().map(Subscription::type_name),
        );

        schema_builder = schema_builder.enable_uploading();

        if self.federation {
            schema_builder = schema_builder.enable_federation();

            if federation_config.is_enabled() {
                let config = federation_config.clone();
                let resolver = self.entity_resolver.clone().unwrap_or_else(|| {
                    std::sync::Arc::new(GrpcEntityResolver::new(client_pool.clone()))
                });

                schema_builder = schema_builder.entity_resolver(move |ctx: ResolverContext<'_>| {
                    let entity_resolvers = resolver.clone();
                    let config = config.clone();

                    FieldFuture::new(async move {
                        let representations = ctx
                            .args
                            .get("representations")
                            .ok_or_else(|| {
                                async_graphql::Error::new("missing representations argument")
                            })?
                            .list()?;

                        let mut results = Vec::new();
                        for repr in representations.iter() {
                            let obj = repr.object()?;

                            let mut representation_map = IndexMap::new();
                            for (key, value) in obj.iter() {
                                representation_map.insert(key.clone(), value.as_value().clone());
                            }

                            let typename = representation_map
                                .get(&Name::new("__typename"))
                                .and_then(|v| match v {
                                    GqlValue::String(s) => Some(s.as_str()),
                                    _ => None,
                                })
                                .ok_or_else(|| {
                                    async_graphql::Error::new(
                                        "missing __typename in representation",
                                    )
                                })?;

                            let entity_config = config.entities.get(typename).ok_or_else(|| {
                                async_graphql::Error::new(format!(
                                    "unknown entity type: {}",
                                    typename
                                ))
                            })?;

                            let entity = entity_resolvers
                                .resolve_entity(entity_config, &representation_map)
                                .await
                                .map_err(|e| async_graphql::Error::new(e.to_string()))?;

                            results.push(
                                FieldValue::value(entity)
                                    .with_type(entity_config.type_name.clone()),
                            );
                        }

                        Ok(Some(FieldValue::list(results)))
                    })
                });
            }
        }

        schema_builder = schema_builder.register(query_root);

        if let Some(mutation) = mutation_root {
            schema_builder = schema_builder.register(mutation);
        }

        if let Some(subscription) = subscription_root {
            schema_builder = schema_builder.register(subscription);
        }

        for (_, en) in registry.enums {
            schema_builder = schema_builder.register(en);
        }
        for (_, input) in registry.input_objects {
            schema_builder = schema_builder.register(input);
        }
        for (type_name, obj) in registry.objects {
            // Apply federation directives if enabled
            let obj = if self.federation {
                federation_config.apply_directives_to_object(obj, &type_name)?
            } else {
                obj
            };
            schema_builder = schema_builder.register(obj);
        }

        let schema = schema_builder
            .finish()
            .map_err(|e| Error::Schema(format!("failed to build schema: {e}")))?;

        Ok(DynamicSchema { inner: schema })
    }
}

impl Default for SchemaBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEDERATION_DESCRIPTOR: &[u8] =
        include_bytes!("generated/federation_example_descriptor.bin");

    #[tokio::test]
    async fn federation_adds_entities_query() {
        let schema = SchemaBuilder::new()
            .with_descriptor_set_bytes(FEDERATION_DESCRIPTOR)
            .enable_federation()
            .build(&GrpcClientPool::new())
            .expect("schema builds");

        let response = schema
            .execute(async_graphql::Request::new(
                "{ _entities(representations: []) { __typename } }",
            ))
            .await;

        assert!(
            response.errors.is_empty(),
            "expected _entities field to exist, got errors: {:?}",
            response.errors
        );

        let data = response.data.into_json().expect("valid JSON response");
        let entities = data
            .get("_entities")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();

        assert!(entities.is_empty(), "expected empty entities list");
    }
}

#[derive(Default)]
struct TypeRegistry {
    objects: HashMap<String, Object>,
    input_objects: HashMap<String, InputObject>,
    enums: HashMap<String, Enum>,
}

/// Per-request memoization to avoid duplicate gRPC calls for identical inputs.
#[derive(Clone, Default)]
pub struct GrpcResponseCache {
    inner: Arc<Mutex<HashMap<GrpcCacheKey, GqlValue>>>,
}

impl GrpcResponseCache {
    pub fn get(&self, key: &GrpcCacheKey) -> Option<GqlValue> {
        self.inner.lock().ok().and_then(|map| map.get(key).cloned())
    }

    pub fn insert(&self, key: GrpcCacheKey, value: GqlValue) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(key, value);
        }
    }
}

#[derive(Hash, Eq, PartialEq, Clone)]
pub struct GrpcCacheKey {
    service: String,
    path: String,
    request: Vec<u8>,
}

impl GrpcCacheKey {
    pub fn new(
        service: &str,
        path: &str,
        request: &DynamicMessage,
    ) -> std::result::Result<Self, prost::EncodeError> {
        Ok(Self {
            service: service.to_string(),
            path: path.to_string(),
            request: request.encode_to_vec(),
        })
    }
}

impl TypeRegistry {
    fn type_name_for_message(desc: &MessageDescriptor) -> String {
        desc.full_name().replace('.', "_")
    }

    fn type_name_for_enum(desc: &EnumDescriptor) -> String {
        desc.full_name().replace('.', "_")
    }

    fn ensure_enum(&mut self, desc: &EnumDescriptor) -> TypeRef {
        let name = Self::type_name_for_enum(desc);
        if !self.enums.contains_key(&name) {
            let mut en = Enum::new(name.clone());
            for value in desc.values() {
                en = en.item(EnumItem::new(value.name()));
            }
            self.enums.insert(name.clone(), en);
        }
        TypeRef::named(name)
    }

    fn ensure_input_object(
        &mut self,
        message: &MessageDescriptor,
        field_ext: &ExtensionDescriptor,
    ) -> TypeRef {
        let name = Self::type_name_for_message(message);
        if self.input_objects.contains_key(&name) {
            return TypeRef::named(name);
        }

        let mut input = InputObject::new(name.clone());
        for field in message.fields() {
            if field_is_omitted(&field, &field_ext) {
                continue;
            }

            let field_name = graphql_field_name(&field, field_ext);
            let required = field_is_required(&field, field_ext);
            let ty = self.input_type_for_field(&field, field_ext, required);
            input = input.field(InputValue::new(field_name, ty));
        }

        self.input_objects.insert(name.clone(), input);
        TypeRef::named(name)
    }

    fn ensure_object(
        &mut self,
        message: &MessageDescriptor,
        field_ext: &ExtensionDescriptor,
    ) -> TypeRef {
        let field_ext = field_ext.clone();
        let name = Self::type_name_for_message(message);
        if self.objects.contains_key(&name) {
            return TypeRef::named(name);
        }

        let mut obj = Object::new(name.clone());
        for field in message.fields() {
            if field_is_omitted(&field, &field_ext) {
                continue;
            }

            let field_name = graphql_field_name(&field, &field_ext);
            let required = field_is_required(&field, &field_ext);
            let ty = self.output_type_for_field(&field, &field_ext, required);
            let field_desc = field.clone();
            let field_name_for_value = field_name.clone();
            let field_ext_for_resolver = field_ext.clone();

            let mut gql_field = Field::new(field_name.clone(), ty, move |ctx| {
                let field_ext = field_ext_for_resolver.clone();
                let field_desc = field_desc.clone();
                let field_name_for_value = field_name_for_value.clone();
                FieldFuture::new(async move {
                    if let Some(parent) = ctx.parent_value.downcast_ref::<DynamicMessage>() {
                        let value = parent.get_field(&field_desc);
                        let gql_value =
                            prost_value_to_graphql(&value, Some(&field_desc), &field_ext)
                                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
                        return Ok(Some(gql_value));
                    }

                    if let Some(GqlValue::Object(map)) = ctx.parent_value.as_value() {
                        if let Some(val) = map.get(&Name::new(field_name_for_value.clone())) {
                            return Ok(Some(val.clone()));
                        }
                    }

                    Ok(Some(GqlValue::Null))
                })
            });

            // Apply federation field directives
            if field_is_external(&field, &field_ext) {
                gql_field = gql_field.directive(async_graphql::dynamic::Directive::new("external"));
            }
            if let Some(requires) = field_requires(&field, &field_ext) {
                gql_field = gql_field.directive(
                    async_graphql::dynamic::Directive::new("requires")
                        .argument("fields", GqlValue::String(requires)),
                );
            }
            if let Some(provides) = field_provides(&field, &field_ext) {
                gql_field = gql_field.directive(
                    async_graphql::dynamic::Directive::new("provides")
                        .argument("fields", GqlValue::String(provides)),
                );
            }

            obj = obj.field(gql_field);
        }

        self.objects.insert(name.clone(), obj);
        TypeRef::named(name)
    }

    fn input_type_for_field(
        &mut self,
        field: &FieldDescriptor,
        field_ext: &ExtensionDescriptor,
        required: bool,
    ) -> TypeRef {
        type_for_field(self, field, field_ext, required, true)
    }

    fn output_type_for_field(
        &mut self,
        field: &FieldDescriptor,
        field_ext: &ExtensionDescriptor,
        required: bool,
    ) -> TypeRef {
        type_for_field(self, field, field_ext, required, false)
    }
}

fn type_for_field(
    registry: &mut TypeRegistry,
    field: &FieldDescriptor,
    field_ext: &ExtensionDescriptor,
    required: bool,
    is_input: bool,
) -> TypeRef {
    let base = match field.kind() {
        Kind::Bool => TypeRef::named(TypeRef::BOOLEAN),
        Kind::String => TypeRef::named(TypeRef::STRING),
        Kind::Bytes => {
            if is_input {
                TypeRef::named(TypeRef::UPLOAD)
            } else {
                TypeRef::named(TypeRef::STRING)
            }
        }
        Kind::Float | Kind::Double => TypeRef::named(TypeRef::FLOAT),
        Kind::Int32 | Kind::Sint32 | Kind::Sfixed32 | Kind::Uint32 | Kind::Fixed32 => {
            TypeRef::named(TypeRef::INT)
        }
        Kind::Int64 | Kind::Sint64 | Kind::Sfixed64 | Kind::Uint64 | Kind::Fixed64 => {
            TypeRef::named(TypeRef::STRING)
        }
        Kind::Enum(en) => registry.ensure_enum(&en),
        Kind::Message(msg) => {
            if is_input {
                registry.ensure_input_object(&msg, field_ext)
            } else {
                registry.ensure_object(&msg, field_ext)
            }
        }
    };

    let base = if required {
        TypeRef::NonNull(Box::new(base))
    } else {
        base
    };

    if field.is_list() {
        TypeRef::List(Box::new(base))
    } else {
        base
    }
}

fn placeholder_query_root() -> Object {
    let ty = TypeRef::NonNull(Box::new(TypeRef::named(TypeRef::BOOLEAN)));
    let field = Field::new("__placeholder", ty, |_| {
        FieldFuture::new(async { Ok(Some(GqlValue::Boolean(true))) })
    });
    Object::new("Query").field(field)
}

#[derive(Clone)]
struct ArgumentSpec {
    name: String,
    ty: TypeRef,
}

impl ArgumentSpec {
    fn into_input_value(self) -> InputValue {
        InputValue::new(self.name, self.ty)
    }
}

#[derive(Clone)]
struct OperationConfig {
    service_name: String,
    grpc_path: String,
    input_desc: MessageDescriptor,
    output_desc: MessageDescriptor,
    request_wrapper_name: Option<String>,
    return_type: TypeRef,
    args: Vec<ArgumentSpec>,
    schema_opts: GraphqlSchema,
    field_ext: ExtensionDescriptor,
}

impl OperationConfig {
    fn new(
        service: &prost_reflect::ServiceDescriptor,
        method: &prost_reflect::MethodDescriptor,
        schema_opts: &GraphqlSchema,
        field_ext: ExtensionDescriptor,
        registry: &mut TypeRegistry,
    ) -> Result<Self> {
        let input_desc = method.input();
        let output_desc = method.output();

        let return_required = schema_opts
            .response
            .as_ref()
            .map(|resp| resp.required)
            .unwrap_or(false);

        let return_type = compute_return_type(
            &output_desc,
            schema_opts.response.as_ref(),
            &field_ext,
            registry,
            return_required,
        );

        let request_wrapper_name = schema_opts
            .request
            .as_ref()
            .filter(|req| !req.name.is_empty())
            .map(|req| req.name.clone());

        let args = build_arguments(&input_desc, &request_wrapper_name, &field_ext, registry);
        let grpc_path = format!("/{}/{}", service.full_name(), method.name());
        let service_name = service.full_name().to_string();

        Ok(Self {
            service_name,
            grpc_path,
            input_desc,
            output_desc,
            request_wrapper_name,
            return_type,
            args,
            schema_opts: schema_opts.clone(),
            field_ext,
        })
    }
}

fn build_field(
    name: String,
    service: &prost_reflect::ServiceDescriptor,
    method: &prost_reflect::MethodDescriptor,
    schema_opts: &GraphqlSchema,
    field_ext: ExtensionDescriptor,
    registry: &mut TypeRegistry,
    client_pool: GrpcClientPool,
) -> Result<Field> {
    let config = OperationConfig::new(service, method, schema_opts, field_ext, registry)?;
    let args = config.args.clone();

    let field = Field::new(name, config.return_type.clone(), move |ctx| {
        let client_pool = client_pool.clone();
        let config = config.clone();
        FieldFuture::new(async move {
            let client = client_pool.get(&config.service_name).ok_or_else(|| {
                async_graphql::Error::new(format!("gRPC client {} not found", config.service_name))
            })?;

            let request_msg = build_request_message(
                &config.input_desc,
                &ctx,
                &config.request_wrapper_name,
                &config.field_ext,
            )?;
            let cache_key =
                GrpcCacheKey::new(&config.service_name, &config.grpc_path, &request_msg).map_err(
                    |e| async_graphql::Error::new(format!("failed to encode request: {e}")),
                )?;

            if let Some(cache) = ctx.data_opt::<GrpcResponseCache>() {
                if let Some(val) = cache.get(&cache_key) {
                    return Ok(Some(val));
                }
            }

            let mut grpc = Grpc::new(client.channel());

            grpc.ready()
                .await
                .map_err(|e| async_graphql::Error::new(format!("gRPC not ready: {e}")))?;

            let codec = ReflectCodec::new(config.output_desc.clone());
            let path: http::uri::PathAndQuery = config
                .grpc_path
                .parse()
                .map_err(|e| async_graphql::Error::new(format!("invalid gRPC path: {e}")))?;

            let response = grpc
                .unary(tonic::Request::new(request_msg), path, codec)
                .await
                .map_err(|e| async_graphql::Error::new(format!("gRPC error: {e}")))?
                .into_inner();

            let value = apply_response_pluck(&response, &config.schema_opts, &config.field_ext)?;
            if let Some(cache) = ctx.data_opt::<GrpcResponseCache>() {
                cache.insert(cache_key, value.clone());
            }
            Ok(Some(value))
        })
    });

    let mut field = field;
    for arg in args {
        field = field.argument(arg.into_input_value());
    }

    Ok(field)
}

fn build_subscription_field(
    name: String,
    service: &prost_reflect::ServiceDescriptor,
    method: &prost_reflect::MethodDescriptor,
    schema_opts: &GraphqlSchema,
    field_ext: ExtensionDescriptor,
    registry: &mut TypeRegistry,
    client_pool: GrpcClientPool,
) -> Result<SubscriptionField> {
    let config = OperationConfig::new(service, method, schema_opts, field_ext, registry)?;
    let args = config.args.clone();

    let field = SubscriptionField::new(name, config.return_type.clone(), move |ctx| {
        let client_pool = client_pool.clone();
        let config = config.clone();
        SubscriptionFieldFuture::new(async move {
            let client = client_pool.get(&config.service_name).ok_or_else(|| {
                async_graphql::Error::new(format!("gRPC client {} not found", config.service_name))
            })?;

            let request_msg = build_request_message(
                &config.input_desc,
                &ctx,
                &config.request_wrapper_name,
                &config.field_ext,
            )?;
            let mut grpc = Grpc::new(client.channel());

            grpc.ready()
                .await
                .map_err(|e| async_graphql::Error::new(format!("gRPC not ready: {e}")))?;
            let codec = ReflectCodec::new(config.output_desc.clone());
            let path: http::uri::PathAndQuery = config
                .grpc_path
                .parse()
                .map_err(|e| async_graphql::Error::new(format!("invalid gRPC path: {e}")))?;

            let response = grpc
                .server_streaming(tonic::Request::new(request_msg), path, codec)
                .await
                .map_err(|e| async_graphql::Error::new(format!("gRPC error: {e}")))?;

            let schema_opts = config.schema_opts.clone();
            let field_ext = config.field_ext.clone();
            let stream = response.into_inner().map(move |item| {
                let msg =
                    item.map_err(|e| async_graphql::Error::new(format!("gRPC error: {e}")))?;
                apply_response_pluck(&msg, &schema_opts, &field_ext)
                    .map_err(|e| async_graphql::Error::new(e.to_string()))
            });

            Ok(stream)
        })
    });

    let mut field = field;
    for arg in args {
        field = field.argument(arg.into_input_value());
    }

    Ok(field)
}

fn build_arguments(
    input_desc: &MessageDescriptor,
    wrapper: &Option<String>,
    field_ext: &ExtensionDescriptor,
    registry: &mut TypeRegistry,
) -> Vec<ArgumentSpec> {
    if let Some(wrapper_name) = wrapper {
        let ty = registry.ensure_input_object(input_desc, field_ext);
        return vec![ArgumentSpec {
            name: wrapper_name.clone(),
            ty,
        }];
    }

    let mut args = Vec::new();
    for field in input_desc.fields() {
        if field_is_omitted(&field, field_ext) {
            continue;
        }
        let required = field_is_required(&field, field_ext);
        let ty = registry.input_type_for_field(&field, field_ext, required);
        let arg_name = graphql_field_name(&field, field_ext);
        args.push(ArgumentSpec { name: arg_name, ty });
    }
    args
}

fn field_is_omitted(field: &FieldDescriptor, field_ext: &ExtensionDescriptor) -> bool {
    decode_extension::<GraphqlField>(&field.options(), field_ext)
        .ok()
        .flatten()
        .map(|f| f.omit)
        .unwrap_or(false)
}

fn field_is_required(field: &FieldDescriptor, field_ext: &ExtensionDescriptor) -> bool {
    decode_extension::<GraphqlField>(&field.options(), field_ext)
        .ok()
        .flatten()
        .map(|f| f.required)
        .unwrap_or(false)
}

fn graphql_field_name(field: &FieldDescriptor, field_ext: &ExtensionDescriptor) -> String {
    decode_extension::<GraphqlField>(&field.options(), field_ext)
        .ok()
        .flatten()
        .and_then(|f| {
            if f.name.is_empty() {
                None
            } else {
                Some(f.name)
            }
        })
        .unwrap_or_else(|| field.name().to_string())
}

fn field_is_external(field: &FieldDescriptor, field_ext: &ExtensionDescriptor) -> bool {
    decode_extension::<GraphqlField>(&field.options(), field_ext)
        .ok()
        .flatten()
        .map(|f| f.external)
        .unwrap_or(false)
}

fn field_requires(field: &FieldDescriptor, field_ext: &ExtensionDescriptor) -> Option<String> {
    decode_extension::<GraphqlField>(&field.options(), field_ext)
        .ok()
        .flatten()
        .and_then(|f| {
            if f.requires.is_empty() {
                None
            } else {
                Some(f.requires)
            }
        })
}

fn field_provides(field: &FieldDescriptor, field_ext: &ExtensionDescriptor) -> Option<String> {
    decode_extension::<GraphqlField>(&field.options(), field_ext)
        .ok()
        .flatten()
        .and_then(|f| {
            if f.provides.is_empty() {
                None
            } else {
                Some(f.provides)
            }
        })
}

fn compute_return_type(
    output_desc: &MessageDescriptor,
    response_opts: Option<&GraphqlResponse>,
    field_ext: &ExtensionDescriptor,
    registry: &mut TypeRegistry,
    required: bool,
) -> TypeRef {
    let pluck_field = response_opts.and_then(|resp| {
        if resp.pluck.is_empty() {
            None
        } else {
            Some(resp.pluck.as_str())
        }
    });

    let base = if let Some(field_name) = pluck_field {
        if let Some(field_desc) = output_desc.get_field_by_name(field_name) {
            registry.output_type_for_field(&field_desc, field_ext, false)
        } else {
            registry.ensure_object(output_desc, field_ext)
        }
    } else {
        registry.ensure_object(output_desc, field_ext)
    };

    if required {
        TypeRef::NonNull(Box::new(base))
    } else {
        base
    }
}

fn build_request_message(
    desc: &MessageDescriptor,
    ctx: &ResolverContext<'_>,
    wrapper: &Option<String>,
    field_ext: &ExtensionDescriptor,
) -> async_graphql::Result<DynamicMessage> {
    if let Some(wrapper_name) = wrapper {
        let wrapper_value = ctx
            .args
            .get(wrapper_name)
            .ok_or_else(|| async_graphql::Error::new(format!("missing argument {wrapper_name}")))?;
        if let GqlValue::Object(obj) = wrapper_value.as_value() {
            return object_to_message(desc, obj, ctx, field_ext);
        } else {
            return Err(async_graphql::Error::new(format!(
                "argument {wrapper_name} must be an object"
            )));
        }
    }

    let mut message = DynamicMessage::new(desc.clone());
    for field in desc.fields() {
        if field_is_omitted(&field, field_ext) {
            continue;
        }
        let arg_name = graphql_field_name(&field, field_ext);
        if let Some(value) = ctx.args.get(&arg_name) {
            let prost_value = graphql_input_to_prost(value.as_value(), &field, ctx, field_ext)?;
            message.set_field(&field, prost_value);
        }
    }
    Ok(message)
}

fn object_to_message(
    desc: &MessageDescriptor,
    values: &IndexMap<Name, GqlValue>,
    ctx: &ResolverContext<'_>,
    field_ext: &ExtensionDescriptor,
) -> async_graphql::Result<DynamicMessage> {
    let mut message = DynamicMessage::new(desc.clone());
    for field in desc.fields() {
        if field_is_omitted(&field, field_ext) {
            continue;
        }
        let arg_name = graphql_field_name(&field, field_ext);
        if let Some(value) = values.get(&Name::new(arg_name)) {
            let prost_value = graphql_input_to_prost(value, &field, ctx, field_ext)?;
            message.set_field(&field, prost_value);
        }
    }
    Ok(message)
}

fn graphql_input_to_prost(
    value: &GqlValue,
    field: &FieldDescriptor,
    ctx: &ResolverContext<'_>,
    field_ext: &ExtensionDescriptor,
) -> async_graphql::Result<Value> {
    if field.is_list() {
        let list = match value {
            GqlValue::List(list) => list,
            _ => return Err(async_graphql::Error::new("expected list")),
        };

        let mut items = Vec::new();
        for v in list {
            items.push(single_input_to_prost(v, field, ctx, field_ext)?);
        }
        return Ok(Value::List(items));
    }

    single_input_to_prost(value, field, ctx, field_ext)
}

fn single_input_to_prost(
    value: &GqlValue,
    field: &FieldDescriptor,
    ctx: &ResolverContext<'_>,
    field_ext: &ExtensionDescriptor,
) -> async_graphql::Result<Value> {
    let kind = field.kind();
    match kind {
        Kind::Bool => match value {
            GqlValue::Boolean(b) => Ok(Value::Bool(*b)),
            _ => Err(async_graphql::Error::new("expected boolean")),
        },
        Kind::String => match value {
            GqlValue::String(s) => Ok(Value::String(s.clone())),
            _ => Err(async_graphql::Error::new("expected string")),
        },
        Kind::Bytes => match value {
            GqlValue::String(s) => {
                if let Some(bytes) = upload_marker_to_bytes(ctx, s)? {
                    return Ok(Value::Bytes(bytes.into()));
                }

                BASE64
                    .decode(s)
                    .map(|b| Value::Bytes(b.into()))
                    .map_err(|e| async_graphql::Error::new(format!("invalid base64: {e}")))
            }
            GqlValue::Binary(b) => Ok(Value::Bytes(b.to_vec().into())),
            _ => Err(async_graphql::Error::new(
                "expected upload or base64 string",
            )),
        },
        Kind::Float | Kind::Double => match value {
            GqlValue::Number(n) => n
                .as_f64()
                .map(Value::F64)
                .ok_or_else(|| async_graphql::Error::new("expected float")),
            _ => Err(async_graphql::Error::new("expected float")),
        },
        Kind::Int32 | Kind::Sint32 | Kind::Sfixed32 => match value {
            GqlValue::Number(n) => n
                .as_i64()
                .map(|v| Value::I32(v as i32))
                .ok_or_else(|| async_graphql::Error::new("expected int")),
            _ => Err(async_graphql::Error::new("expected int")),
        },
        Kind::Uint32 | Kind::Fixed32 => match value {
            GqlValue::Number(n) => n
                .as_u64()
                .map(|v| Value::U32(v as u32))
                .ok_or_else(|| async_graphql::Error::new("expected unsigned int")),
            _ => Err(async_graphql::Error::new("expected unsigned int")),
        },
        Kind::Int64 | Kind::Sint64 | Kind::Sfixed64 => match value {
            GqlValue::String(s) => s
                .parse::<i64>()
                .map(Value::I64)
                .map_err(|e| async_graphql::Error::new(format!("invalid i64: {e}"))),
            GqlValue::Number(n) => n
                .as_i64()
                .map(Value::I64)
                .ok_or_else(|| async_graphql::Error::new("expected 64-bit int")),
            _ => Err(async_graphql::Error::new("expected 64-bit int (as string)")),
        },
        Kind::Uint64 | Kind::Fixed64 => match value {
            GqlValue::String(s) => s
                .parse::<u64>()
                .map(Value::U64)
                .map_err(|e| async_graphql::Error::new(format!("invalid u64: {e}"))),
            GqlValue::Number(n) => n
                .as_u64()
                .map(Value::U64)
                .ok_or_else(|| async_graphql::Error::new("expected 64-bit uint")),
            _ => Err(async_graphql::Error::new(
                "expected 64-bit uint (as string)",
            )),
        },
        Kind::Enum(en) => {
            let name_str = match value {
                GqlValue::Enum(name) => Some(name.as_str()),
                GqlValue::String(name) => Some(name.as_str()),
                _ => None,
            }
            .ok_or_else(|| async_graphql::Error::new("expected enum value"))?;

            let num = en
                .get_value_by_name(name_str)
                .map(|v| v.number())
                .ok_or_else(|| async_graphql::Error::new("invalid enum value"))?;
            Ok(Value::EnumNumber(num))
        }
        Kind::Message(msg) => match value {
            GqlValue::Object(obj) => {
                object_to_message(&msg, obj, ctx, field_ext).map(Value::Message)
            }
            _ => Err(async_graphql::Error::new("expected object")),
        },
    }
}

const UPLOAD_REF_PREFIX: &str = "#__graphql_file__:";

fn upload_marker_to_bytes(
    ctx: &ResolverContext<'_>,
    marker: &str,
) -> async_graphql::Result<Option<Vec<u8>>> {
    let Some(rest) = marker.strip_prefix(UPLOAD_REF_PREFIX) else {
        return Ok(None);
    };

    let idx = rest.parse::<usize>().map_err(|e| {
        async_graphql::Error::new(format!("invalid upload reference {marker}: {e}"))
    })?;

    let upload: &UploadValue = ctx
        .query_env
        .uploads
        .get(idx)
        .ok_or_else(|| async_graphql::Error::new(format!("upload index {idx} not found")))?;

    let reader = upload
        .try_clone()
        .map_err(|e| async_graphql::Error::new(format!("failed to clone upload: {e}")))?;
    let mut bytes = Vec::new();
    reader
        .into_read()
        .read_to_end(&mut bytes)
        .map_err(|e| async_graphql::Error::new(format!("failed to read upload: {e}")))?;

    Ok(Some(bytes))
}

fn prost_value_to_graphql(
    value: &Value,
    field: Option<&FieldDescriptor>,
    field_ext: &ExtensionDescriptor,
) -> Result<GqlValue> {
    let kind = field.map(|f| f.kind());
    Ok(match value {
        Value::Bool(b) => GqlValue::Boolean(*b),
        Value::I32(v) => GqlValue::from(*v),
        Value::I64(v) => GqlValue::from(*v),
        Value::U32(v) => GqlValue::from(*v as i64),
        Value::U64(v) => GqlValue::from(*v as i64),
        Value::F32(v) => GqlValue::from(*v),
        Value::F64(v) => GqlValue::from(*v),
        Value::String(s) => GqlValue::from(s.clone()),
        Value::Bytes(b) => GqlValue::from(BASE64.encode(b)),
        Value::EnumNumber(num) => {
            if let Some(Kind::Enum(en)) = kind {
                if let Some(val) = en.get_value(*num) {
                    GqlValue::Enum(Name::new(val.name()))
                } else {
                    GqlValue::from(*num)
                }
            } else {
                GqlValue::from(*num)
            }
        }
        Value::Message(msg) => {
            dynamic_message_to_value(msg, field_ext).unwrap_or_else(|_| GqlValue::Null)
        }
        Value::List(list) => {
            let items = list
                .iter()
                .map(|v| prost_value_to_graphql(v, field, field_ext))
                .collect::<Result<Vec<_>>>()?;
            GqlValue::List(items)
        }
        Value::Map(map) => {
            let mut obj = IndexMap::new();
            for (k, v) in map {
                obj.insert(
                    Name::new(map_key_to_string(k)),
                    prost_value_to_graphql(v, field, field_ext)?,
                );
            }
            GqlValue::Object(obj)
        }
    })
}

fn dynamic_message_to_value(
    message: &DynamicMessage,
    field_ext: &ExtensionDescriptor,
) -> Result<GqlValue> {
    let mut map = IndexMap::new();
    for field_desc in message.descriptor().fields() {
        if field_is_omitted(&field_desc, field_ext) {
            continue;
        }
        let value = message.get_field(&field_desc);
        let gql_value = prost_value_to_graphql(&value, Some(&field_desc), field_ext)?;
        let name = graphql_field_name(&field_desc, field_ext);
        map.insert(Name::new(name), gql_value);
    }
    Ok(GqlValue::Object(map))
}

fn apply_response_pluck(
    response: &DynamicMessage,
    schema_opts: &GraphqlSchema,
    field_ext: &ExtensionDescriptor,
) -> Result<GqlValue> {
    if let Some(resp_opts) = &schema_opts.response {
        if !resp_opts.pluck.is_empty() {
            if let Some(field_desc) = response.descriptor().get_field_by_name(&resp_opts.pluck) {
                let val = response.get_field(&field_desc);
                return prost_value_to_graphql(&val, Some(&field_desc), field_ext);
            }
        }
    }

    dynamic_message_to_value(response, field_ext)
}

fn map_key_to_string(key: &MapKey) -> String {
    match key {
        MapKey::Bool(b) => b.to_string(),
        MapKey::I32(v) => v.to_string(),
        MapKey::I64(v) => v.to_string(),
        MapKey::U32(v) => v.to_string(),
        MapKey::U64(v) => v.to_string(),
        MapKey::String(s) => s.clone(),
    }
}

fn decode_extension<T: Message + Default>(
    opts: &DynamicMessage,
    ext: &ExtensionDescriptor,
) -> Result<Option<T>> {
    if !opts.has_extension(ext) {
        return Ok(None);
    }
    let val = opts.get_extension(ext);
    if let Value::Message(msg) = val.as_ref() {
        T::decode(msg.encode_to_vec().as_slice())
            .map(Some)
            .map_err(|e| Error::Schema(format!("failed to decode extension: {e}")))
    } else {
        Ok(None)
    }
}

/// Codec for dynamic protobuf messages (avoids the `Default` bound on `ProstCodec` decode type).
struct ReflectCodec {
    response_desc: MessageDescriptor,
}

impl ReflectCodec {
    fn new(response_desc: MessageDescriptor) -> Self {
        Self { response_desc }
    }
}

impl Codec for ReflectCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = ReflectEncoder;
    type Decoder = ReflectDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        ReflectEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        ReflectDecoder {
            desc: self.response_desc.clone(),
        }
    }
}

struct ReflectDecoder {
    desc: MessageDescriptor,
}

struct ReflectEncoder;

impl Decoder for ReflectDecoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn decode(
        &mut self,
        buf: &mut DecodeBuf<'_>,
    ) -> std::result::Result<Option<Self::Item>, Self::Error> {
        if !Buf::has_remaining(buf) {
            return Ok(None);
        }
        let len = Buf::remaining(buf);
        let bytes = buf.copy_to_bytes(len);
        Ok(DynamicMessage::decode(self.desc.clone(), bytes)
            .map(Some)
            .map_err(|e| Status::internal(format!("decode error: {e}")))?)
    }
}

impl Encoder for ReflectEncoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn encode(
        &mut self,
        item: Self::Item,
        buf: &mut EncodeBuf<'_>,
    ) -> std::result::Result<(), Self::Error> {
        item.encode(buf)
            .map_err(|e| Status::internal(format!("encode error: {e}")))
    }
}
