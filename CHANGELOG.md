# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial Rust implementation of grpc-graphql-gateway
- Support for GraphQL queries from gRPC unary calls
- Support for GraphQL mutations from gRPC unary calls
- Support for GraphQL subscriptions from gRPC streaming calls
- WebSocket support for GraphQL subscriptions (graphql-ws protocol)
- Middleware system for authentication, logging, etc.
- gRPC client connection pooling
- Comprehensive error handling with GraphQL error formatting
- Axum-based HTTP server with routing
- Example greeter service demonstrating all features
- Full async/await support with Tokio runtime
- **GraphQL Federation v2 support**
  - Entity definitions via `graphql.entity` proto option
  - Entity key support (single and composite keys)
  - Entity extensions with `@extends` directive
  - Field-level federation directives: `@external`, `@requires`, `@provides`
  - Automatic `_entities` query generation for entity resolution
  - `EntityResolver` trait for custom entity resolution logic
  - Federation example in `proto/federation_example.proto`
  - Comprehensive federation documentation

### Architecture
- `gateway` module - Main orchestration and builder
- `runtime` module - ServeMux for handling HTTP and WebSocket requests
- `schema` module - GraphQL schema building from gRPC services
- `grpc_client` module - gRPC client connection management
- `middleware` module - Middleware support (CORS, auth, logging)
- `error` module - Error types and GraphQL error formatting
- `types` module - Core type definitions

### Performance
- Zero-cost abstractions leveraging Rust's type system
- Compile-time guarantees for memory safety
- Efficient async I/O with Tokio
- Connection pooling for gRPC clients

## [0.1.0] - 2024-12-02

### Added
- Initial release
- Basic gRPC to GraphQL gateway functionality
- Query and Mutation support
- Subscription support via WebSocket
- Middleware system
- Example implementation

[Unreleased]: https://github.com/Protocol-Lattice/grpc-graphql-gateway-rs/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Protocol-Lattice/grpc-graphql-gateway-rs/releases/tag/v0.1.0
