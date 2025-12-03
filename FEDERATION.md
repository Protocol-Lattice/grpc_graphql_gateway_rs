# GraphQL Federation Implementation Summary

## Overview

GraphQL Federation support has been successfully added to `grpc-graphql-gateway-rs`. This enables the gateway to participate in a federated GraphQL architecture following the Apollo Federation v2 specification.

## Changes Made

### 1. Proto File Extensions (`proto/graphql.proto`)

Added federation-specific protocol buffer messages and extensions:

- **`GraphqlEntity`** message for entity configuration:
  - `keys`: List of key field sets (supports single and composite keys)
  - `extend`: Boolean flag for entity extensions
  - `resolvable`: Whether this service can resolve the entity

- **`GraphqlField`** additions for field-level directives:
  - `external`: Mark fields defined in other services  
  - `requires`: Specify required fields from other services
  - `provides`: Indicate fields provided to the supergraph

- **New extension**: `google.protobuf.MessageOptions.entity` for entity configuration

### 2. Federation Module (`src/federation.rs`)

New module implementing core federation functionality:

#### Key Types:
- **`FederationConfig`**: Stores entity configurations extracted from descriptors
- **`EntityConfig`**: Configuration for individual federated entities
- **`EntityResolver` trait**: Interface for custom entity resolution logic
- **`GrpcEntityResolver`**: Default implementation (returns representations as-is)

#### Key Functions:
- `from_descriptor_pool()`: Extract federation config from protobuf descriptors
- `build_entities_field()`: Generate `_entities` query field
- `apply_directives_to_object()`: Apply @key and @extends directives to types

### 3. Schema Integration (`src/schema.rs`)

Updated schema builder to support federation:

- Import and use `FederationConfig` and `GrpcEntityResolver`
- Load entity extension when federation is enabled
- Extract federation configuration from descriptor pool
- Add `_entities` field to Query type when entities exist
- Apply federation directives to object types during registration
- Apply field-level directives (@external, @requires, @provides)

Helper functions added:
- `field_is_external()`: Check if field is marked external
- `field_requires()`: Get @requires directive value
- `field_provides()`: Get @provides directive value

### 4. Public API Updates (`src/lib.rs`)

Exported new federation types:
- `EntityResolver`
- `FederationConfig`  
- `GrpcEntityResolver`

### 5. Documentation

#### README.md (`README.md`)
Comprehensive federation documentation including:
- How to enable federation
- Defining entities with @key
- Entity extensions with @extends
- Field-level directives (@external, @requires, @provides)
- Entity resolution patterns
- Federation schema features checklist
- Federated microservices example
- Best practices

#### Changelog (`CHANGELOG.md`)
Added detailed changelog entry for federation feature including:
- Entity definitions via proto options
- Support for single and composite keys
- Entity extensions
- All field-level directives
- Automatic _entities query generation
- EntityResolver trait
- Example and documentation

### 6. Examples

Created `proto/federation_example.proto` demonstrating:
- User entity (defining service)
- Product entity (with relationships)
- Review entity (extending multiple entities)
- UserExtension (service extending User from another service)
- Proper use of all federation directives

## Federation Features Supported

✅ Apollo Federation v2 specification  
✅ Entity definitions with `@key` directive  
✅ Single field keys (e.g., `keys: "id"`)  
✅ Composite keys (e.g., `keys: "orgId userId"`)  
✅ Multiple keys per entity  
✅ Entity extensions with `@extends` directive  
✅ `@external` field directive  
✅ `@requires` field directive  
✅ `@provides` field directive  
✅ Automatic `_entities` query generation  
✅ `_service { sdl }` query (via async-graphql)  
✅ Entity type union registration  
✅ EntityResolver trait for custom resolution  

## Usage Example

```rust
use grpc_graphql_gateway::{Gateway, GrpcClient};

const DESCRIPTORS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/graphql_descriptor.bin"));

#[tokio::main]
async fn main() -> Result<()> {
    let gateway = Gateway::builder()
        .with_descriptor_set_bytes(DESCRIPTORS)
        .enable_federation()  // Enable federation support
        .add_grpc_client("service", GrpcClient::builder("http://localhost:50051").lazy(true).connect_lazy()?)
        .build()?;
    
    gateway.serve("0.0.0.0:8888").await
}
```

## Proto Example

```protobuf
message User {
  option (graphql.entity) = {
    keys: "id"
    resolvable: true
  };
  
  string id = 1 [(graphql.field) = { required: true }];
  string email = 2;
  string name = 3;
}

message Product {
  option (graphql.entity) = {
    extend: true
    keys: "upc"
  };
  
  string upc = 1 [(graphql.field) = { 
    external: true 
    required: true 
  }];
  
  User created_by = 2 [(graphql.field) = {
    provides: "id name"
  }];
}
```

## Running with Apollo Router

Compose a supergraph for the example subgraph and run Apollo Router in front of it:

```bash
cargo run --bin federation
./examples/federation/compose_supergraph.sh
router --supergraph examples/federation/supergraph.graphql --dev
```

## Curl examples

Through the router (`http://127.0.0.1:4000/`):

```bash
# Fetch a user entity
curl -X POST http://127.0.0.1:4000/ \
  -H 'content-type: application/json' \
  -d '{"query":"{ user(id:\"u1\") { id email name } }"}'

# Fetch a product with nested creator
curl -X POST http://127.0.0.1:4000/ \
  -H 'content-type: application/json' \
  -d "{\"query\":\"{ product(upc:\\\"apollo-1\\\") { upc name price createdBy { id name } } }\"}"
```

Resolve an entity directly against the subgraph (`http://127.0.0.1:8891/graphql` for the User subgraph). Apollo Router does not expose `_entities` on the supergraph API, but the gateway does:

```bash
curl -X POST http://127.0.0.1:8891/graphql \
  -H 'content-type: application/json' \
  -d '{"query":"{ _entities(representations:[{ __typename:\"federation_example_User\", id:\"u1\" }]) { ... on federation_example_User { id email name } } }"}'
```

## Testing

All tests pass successfully:
- ✅ `test_federation_config_new()` - Federation config initialization
- ✅ `test_entity_config_composite_keys()` - Composite key parsing
- ✅ All existing gateway tests continue to pass

## Future Enhancements

While the core federation support is complete, potential enhancements include:

1. **Full Entity Resolution**: Implement a complete GrpcEntityResolver that:
   - Maps entity types to gRPC resolver methods
   - Builds gRPC requests from representations
   - Calls appropriate gRPC services
   - Transforms responses to GraphQL values

2. **Federation Validation**: Add validation for:
   - Key fields exist in entity types
   - External fields are not resolved locally
   - Required fields are available in context
   - Circular dependencies

3. **Supergraph Integration**: Add tooling for:
   - Generating subgraph SDL
   - Publishing to Apollo Studio
   - Schema composition validation

4. **Performance Optimizations**:
   - Batch entity resolution
   - Entity caching
   - Dataloader pattern for N+1 prevention

## Compatibility

- **Rust Edition**: 2021
- **async-graphql**: 7.0 (with dynamic-schema feature)
- **Apollo Federation**: v2 compatible
- **GraphQL**: Spec compliant

## Notes

- The `enable_federation()` method must be called on the gateway builder to activate federation features
- Entity resolution currently returns representations as-is; implement custom `EntityResolver` for production use
- Federation directives are automatically applied based on proto annotations
- The `_entities` query is only added when at least one entity is defined
- All federation features work with existing non-federated schemas (backwards compatible)
