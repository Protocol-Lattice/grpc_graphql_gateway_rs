# grpc-graphql-gateway-rs

A Rust implementation of [grpc-graphql-gateway](https://github.com/ysugimoto/grpc-graphql-gateway) - generates GraphQL execution code from gRPC services.

![Rust Version](https://img.shields.io/badge/rust-1.75%2B-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

## 🎯 Motivation

This is a Rust port of the popular Go-based `grpc-graphql-gateway`. It provides a bridge between gRPC services and GraphQL, allowing you to:

- **GraphQL** - Aggregate multiple resources into one HTTP request, perfect for BFF (Backend-for-Frontend)
- **gRPC** - Leverage Protocol Buffers' simplicity and HTTP/2 performance
- **Single Source of Truth** - Define your API once in Protocol Buffers, get both gRPC and GraphQL

## ✨ Features

- ✅ **Query Support** - Expose gRPC unary calls as GraphQL queries
- ✅ **Mutation Support** - Expose gRPC unary calls as GraphQL mutations
- ✅ **Subscription Support** - Stream gRPC responses via GraphQL subscriptions over WebSocket
- ✅ **Type Mapping** - Automatic conversion between Protocol Buffers types and GraphQL types
- ✅ **Middleware Support** - Add authentication, logging, and custom middleware
- ✅ **Error Handling** - Comprehensive error handling with proper GraphQL error formatting
- ✅ **Async/Await** - Built on Tokio for high-performance async I/O
- ✅ **WebSocket** - Full GraphQL subscription support over WebSocket (graphql-ws protocol)

## 📦 Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
grpc-graphql-gateway = "0.1"
tonic = "0.12"
tokio = { version = "1.0", features = ["full"] }
```

## 🚀 Quick Start

### 1. Define Your gRPC Service

```protobuf
// greeter.proto
syntax = "proto3";

package greeter;

service Greeter {
  rpc SayHello (HelloRequest) returns (HelloReply);
  rpc SayGoodbye (GoodbyeRequest) returns (GoodbyeReply);
  rpc StreamGreetings (HelloRequest) returns (stream HelloReply);
}

message HelloRequest {
  string name = 1;
}

message HelloReply {
  string message = 1;
}

message GoodbyeRequest {
  string name = 1;
}

message GoodbyeReply {
  string message = 1;
}
```

### 2. Implement Your gRPC Service

```rust
use tonic::{Request, Response, Status};
use greeter::greeter_server::{Greeter, GreeterServer};

#[derive(Default)]
pub struct GreeterService;

#[tonic::async_trait]
impl Greeter for GreeterService {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let name = request.into_inner().name;
        Ok(Response::new(HelloReply {
            message: format!("Hello, {}!", name),
        }))
    }

    async fn say_goodbye(
        &self,
        request: Request<GoodbyeRequest>,
    ) -> Result<Response<GoodbyeReply>, Status> {
        let name = request.into_inner().name;
        Ok(Response::new(GoodbyeReply {
            message: format!("Goodbye, {}!", name),
        }))
    }

    type StreamGreetingsStream = ReceiverStream<Result<HelloReply, Status>>;

    async fn stream_greetings(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<Self::StreamGreetingsStream>, Status> {
        let name = request.into_inner().name;
        let (tx, rx) = mpsc::channel(4);

        tokio::spawn(async move {
            let greetings = vec![
                format!("Hello, {}!", name),
                format!("Greetings, {}!", name),
                format!("Good day, {}!", name),
            ];

            for greeting in greetings {
                tx.send(Ok(HelloReply { message: greeting }))
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}
```

### 3. Create GraphQL Gateway

```rust
use grpc_graphql_gateway::{Gateway, GrpcClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create gRPC client pool
    let grpc_client = GrpcClient::new("http://localhost:50051").await?;

    // Build GraphQL gateway
    let gateway = Gateway::builder()
        .add_grpc_client("greeter", grpc_client)
        .build();

    // Start HTTP server with GraphQL endpoint
    let app = gateway.into_router();

    let addr = "0.0.0.0:8888".parse()?;
    println!("GraphQL server listening on http://{}/graphql", addr);

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
```

### 4. Query via GraphQL

```bash
# Query
curl -X POST http://localhost:8888/graphql \
  -H "Content-Type: application/json" \
  -d '{
    "query": "{ hello(name: \"World\") { message } }"
  }'

# Response
{
  "data": {
    "hello": {
      "message": "Hello, World!"
    }
  }
}
```

### 5. Subscribe via WebSocket

```bash
# Using wscat
wscat -c ws://localhost:8888/graphql -s graphql-ws

# Send subscription
{"id":"1","type":"subscribe","payload":{"query":"subscription { streamHello(name: \"Rust\") { message } }"}}

# Receive streaming responses
{"id":"1","type":"data","payload":{"data":{"streamHello":{"message":"Hello, Rust!"}}}}
{"id":"1","type":"data","payload":{"data":{"streamHello":{"message":"Greetings, Rust!"}}}}
{"id":"1","type":"complete"}
```

## 🏗️ Architecture

```
┌─────────────┐
│   Client    │
└──────┬──────┘
       │ GraphQL Query/Mutation/Subscription
       ▼
┌─────────────────────────────────┐
│  GraphQL Gateway (This Crate)  │
│  ┌───────────────────────────┐ │
│  │   async-graphql Schema    │ │
│  └───────────┬───────────────┘ │
│              │                  │
│  ┌───────────▼───────────────┐ │
│  │   gRPC Client Connection  │ │
│  └───────────────────────────┘ │
└──────────────┬──────────────────┘
               │ gRPC Request
               ▼
        ┌──────────────┐
        │ gRPC Service │
        └──────────────┘
```

## 📚 Examples

Check out the [examples](./examples) directory for complete working examples:

- **[greeter](./examples/greeter)** - Simple hello world example with queries
- **[starwars](./examples/starwars)** - Complex example with mutations and nested resolvers
- **[subscriptions](./examples/subscriptions)** - Streaming data with GraphQL subscriptions

## 🔧 Advanced Usage

### Custom Middleware

```rust
use grpc_graphql_gateway::middleware::{Middleware, Context};

struct AuthMiddleware;

#[async_trait]
impl Middleware for AuthMiddleware {
    async fn call(&self, ctx: &mut Context) -> Result<(), Error> {
        // Validate authorization header
        if let Some(token) = ctx.request.headers().get("authorization") {
            // Validate token...
            Ok(())
        } else {
            Err(Error::Unauthorized)
        }
    }
}

let gateway = Gateway::builder()
    .add_middleware(AuthMiddleware)
    .build();
```

### Type Mapping

The gateway automatically maps Protocol Buffer types to GraphQL types:

| Protobuf Type | GraphQL Type |
|---------------|--------------|
| `string`      | `String`     |
| `int32`       | `Int`        |
| `int64`       | `String`     |
| `float`       | `Float`      |
| `double`      | `Float`      |
| `bool`        | `Boolean`    |
| `bytes`       | `String`     |
| `repeated`    | `[Type]`     |
| `message`     | `Object`     |
| `enum`        | `Enum`       |

## 🔄 Comparison with Go Implementation

| Feature | Go Version | Rust Version |
|---------|-----------|--------------|
| Query Support | ✅ | ✅ |
| Mutation Support | ✅ | ✅ |
| Subscription Support | ✅ | ✅ |
| Middleware | ✅ | ✅ |
| Performance | Good | **Excellent** |
| Memory Safety | Runtime checks | **Compile-time guarantees** |
| Async/Await | goroutines | Tokio tasks |

## 🎯 Design Differences

While maintaining API compatibility with the Go version, the Rust implementation provides:

- **Type Safety**: Leverage Rust's type system for compile-time guarantees
- **Zero-Cost Abstractions**: No runtime overhead for abstractions
- **Memory Efficiency**: No garbage collection pauses
- **Fearless Concurrency**: Safe concurrent programming with Rust's ownership model

## 🤝 Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -am 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## 📝 License

MIT License - see the [LICENSE](LICENSE) file for details

## 🙏 Acknowledgments

- [ysugimoto/grpc-graphql-gateway](https://github.com/ysugimoto/grpc-graphql-gateway) - Original Go implementation
- [async-graphql](https://github.com/async-graphql/async-graphql) - Powerful GraphQL server library
- [tonic](https://github.com/hyperium/tonic) - gRPC implementation in Rust

## 📖 Related Projects

- [grpc-gateway](https://github.com/grpc-ecosystem/grpc-gateway) - gRPC to REST gateway
- [rs-utcp](https://github.com/Protocol-Lattice/rs-utcp) - Universal Tool Communication Protocol in Rust
- [go-utcp](https://github.com/Protocol-Lattice/go-utcp) - Universal Tool Communication Protocol in Go
