# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - 2025-12-04

### Added
- **EntityDataLoader**: Built-in DataLoader for batching entity resolution requests to prevent N+1 query problems.
- **Documentation**: Comprehensive Rustdoc documentation across all public APIs and internal modules.
- **Examples**: Enhanced federation example demonstrating `EntityDataLoader` usage and batching.

### Improved
- **README**: Completely rewritten README with better structure, modern formatting, and detailed examples.
- **Federation**: Added `batch_resolve_entities` support to `EntityResolver` trait.

## [0.1.1] - 2025-12-04

### Added
- **Federation v2**: Full support for `@shareable` directive in proto options and schema generation.
- **Entity Resolution**: Production-ready `GrpcEntityResolver` with `EntityResolverMapping` for mapping entities to gRPC methods.
- **Builder**: `GrpcEntityResolverBuilder` for easier configuration of entity resolvers.

### Fixed
- **Composition**: Resolved `INVALID_FIELD_SHARING` errors by correctly applying `@shareable` directive.

## [0.1.0] - 2024-12-02

### Added
- Initial release
- Basic gRPC to GraphQL gateway functionality
- Query and Mutation support
- Subscription support via WebSocket
- Middleware system
- Example implementation
- **GraphQL Federation v2 support**
  - Entity definitions via `graphql.entity` proto option
  - Entity key support (single and composite keys)
  - Entity extensions with `@extends` directive
  - Field-level federation directives: `@external`, `@requires`, `@provides`
  - Automatic `_entities` query generation for entity resolution
  - `EntityResolver` trait for custom entity resolution logic

[0.1.2]: https://github.com/Protocol-Lattice/grpc-graphql-gateway-rs/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/Protocol-Lattice/grpc-graphql-gateway-rs/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Protocol-Lattice/grpc-graphql-gateway-rs/releases/tag/v0.1.0
