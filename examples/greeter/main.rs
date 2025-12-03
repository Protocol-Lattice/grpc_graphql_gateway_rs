use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::StreamExt;
use grpc_graphql_gateway::{Gateway, GrpcClient};
use tokio::fs;
use tokio::sync::RwLock;
use tokio_stream::wrappers::IntervalStream;
use tokio_stream::Stream;
use tonic::{transport::Server, Request, Response, Status};
use tracing::info;

pub mod greeter {
    include!("../../src/generated/greeter.rs");
}

use greeter::greeter_server::{Greeter, GreeterServer};
use greeter::{
    GetUserRequest, GreetMeta, HelloReply, HelloRequest, UpdateGreetingRequest, UploadAvatarReply,
    UploadAvatarRequest, UploadAvatarsReply, UploadAvatarsRequest, User,
};

const DESCRIPTORS: &[u8] = include_bytes!("../../src/generated/greeter_descriptor.bin");
const GRPC_ADDR: &str = "127.0.0.1:50051";
const GQL_ADDR: &str = "127.0.0.1:8888";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .compact()
        .init();

    let grpc_addr: SocketAddr = GRPC_ADDR.parse()?;
    let gql_addr: SocketAddr = GQL_ADDR.parse()?;

    let greeter = ExampleGreeter::default();

    print_examples(gql_addr);

    let grpc = tokio::spawn(run_grpc_server(greeter.clone(), grpc_addr));
    let gateway = tokio::spawn(run_gateway(gql_addr));

    let (grpc_res, gateway_res) = tokio::join!(grpc, gateway);
    grpc_res??;
    gateway_res??;

    Ok(())
}

async fn run_grpc_server(greeter: ExampleGreeter, addr: SocketAddr) -> Result<()> {
    info!("gRPC Greeter listening on {}", addr);

    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await?;

    Ok(())
}

async fn run_gateway(addr: SocketAddr) -> Result<()> {
    info!(
        "GraphQL gateway listening on http://{}/graphql (ws://{}/graphql/ws for subscriptions)",
        addr, addr
    );

    let client = GrpcClient::builder(format!("http://{GRPC_ADDR}"))
        .lazy(false)
        .connect()
        .await?;

    Gateway::builder()
        .with_descriptor_set_bytes(DESCRIPTORS)
        .add_grpc_client("greeter.Greeter", client)
        .serve(addr.to_string())
        .await?;

    Ok(())
}

fn print_examples(addr: SocketAddr) {
    println!("GraphQL endpoint: http://{}/graphql", addr);
    println!("WebSocket endpoint: ws://{}/graphql/ws", addr);
    // curl (query): curl -X POST http://127.0.0.1:8888/graphql -H 'content-type: application/json' -d '{"query":"{ hello(name:\"GraphQL\"){ message } }"}'
    // curl (mutation): curl -X POST http://127.0.0.1:8888/graphql -H 'content-type: application/json' -d '{"query":"mutation { updateGreeting(input:{ name:\"GraphQL\", salutation:\"Howdy\" }) { message } }"}'
    // curl (resolver): curl -X POST http://127.0.0.1:8888/graphql -H 'content-type: application/json' -d '{"query":"{ user(id:\"demo\"){ id displayName trusted } }"}'
    // subscription (graphql-transport-ws):
    //   websocat -H="Sec-WebSocket-Protocol: graphql-transport-ws" --protocol graphql-transport-ws ws://127.0.0.1:8888/graphql/ws
    //   # then type/paste:
    //   {"type":"connection_init","payload":{}}
    //   {"id":"1","type":"subscribe","payload":{"query":"subscription { streamHello(name:\"GraphQL\"){ message } }"}}
    //   # server replies with {"type":"connection_ack"} then pushes data payloads.
    println!("Try these operations once the servers are up:");
    println!(
        "  query {{ hello(name:\"GraphQL\") {{ message meta {{ correlationId from {{ id displayName trusted }} }} }} }}"
    );
    println!(
        "  mutation {{ updateGreeting(input:{{ name:\"GraphQL\", salutation:\"Howdy\" }}) {{ message }} }}"
    );
    println!(
        "  subscription {{ streamHello(name:\"GraphQL\") {{ message meta {{ correlationId }} }} }}"
    );
    println!("  query {{ user(id:\"demo\") {{ id displayName trusted }} }}");
    println!("  # Upload (multipart): see README for the curl example");
    println!("  # Multi-upload (multipart): see README for the curl example");
}

#[derive(Clone)]
struct ExampleGreeter {
    greeting: Arc<RwLock<String>>,
    users: Arc<RwLock<HashMap<String, User>>>,
}

impl Default for ExampleGreeter {
    fn default() -> Self {
        let mut users = HashMap::new();
        users.insert(
            "demo".to_string(),
            User {
                id: "demo".to_string(),
                display_name: "Demo User".to_string(),
                trusted: true,
            },
        );
        users.insert(
            "admin".to_string(),
            User {
                id: "admin".to_string(),
                display_name: "Admin User".to_string(),
                trusted: false,
            },
        );

        Self {
            greeting: Arc::new(RwLock::new("Hello".to_string())),
            users: Arc::new(RwLock::new(users)),
        }
    }
}

#[tonic::async_trait]
impl Greeter for ExampleGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let req = request.into_inner();
        let reply = self.build_reply(normalize_name(req.name), "query").await;
        Ok(Response::new(reply))
    }

    async fn update_greeting(
        &self,
        request: Request<UpdateGreetingRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let req = request.into_inner();
        {
            let mut greeting = self.greeting.write().await;
            *greeting = req.greeting.clone();
        }

        let reply = self.build_reply(normalize_name(req.name), "mutation").await;
        Ok(Response::new(reply))
    }

    type StreamHellosStream =
        Pin<Box<dyn Stream<Item = Result<HelloReply, Status>> + Send + 'static>>;

    async fn stream_hellos(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<Self::StreamHellosStream>, Status> {
        let name = normalize_name(request.into_inner().name);
        let greeting = self.greeting.clone();
        let users = self.users.clone();

        let stream = IntervalStream::new(tokio::time::interval(Duration::from_secs(1)))
            .enumerate()
            .take(5)
            .then(move |(idx, _)| {
                let greeting = greeting.clone();
                let users = users.clone();
                let name = name.clone();
                async move {
                    let greeting = greeting.read().await.clone();
                    let user = users.read().await.get("demo").cloned();

                    Ok(HelloReply {
                        message: format!("{greeting}, {name}! (#{})", idx + 1),
                        meta: Some(GreetMeta {
                            correlation_id: format!("stream-{idx}"),
                            from: user,
                        }),
                    })
                }
            });

        Ok(Response::new(Box::pin(stream) as Self::StreamHellosStream))
    }

    async fn resolve_user(
        &self,
        request: Request<GetUserRequest>,
    ) -> Result<Response<User>, Status> {
        let req = request.into_inner();
        if let Some(user) = self.lookup_user(&req.id).await {
            return Ok(Response::new(user));
        }

        Err(Status::not_found(format!("user {} not found", req.id)))
    }

    async fn upload_avatar(
        &self,
        request: Request<UploadAvatarRequest>,
    ) -> Result<Response<UploadAvatarReply>, Status> {
        let req = request.into_inner();
        if self.lookup_user(&req.user_id).await.is_none() {
            return Err(Status::not_found(format!("user {} not found", req.user_id)));
        }

        let mut path = std::env::temp_dir();
        path.push(format!(
            "greeter_avatar_{}.bin",
            safe_filename(&req.user_id)
        ));
        fs::write(&path, &req.avatar)
            .await
            .map_err(|e| Status::internal(format!("failed to write avatar: {e}")))?;
        info!("stored avatar for {} at {}", req.user_id, path.display());

        let size = req.avatar.len() as u64;
        let reply = UploadAvatarReply {
            user_id: req.user_id,
            size,
        };
        Ok(Response::new(reply))
    }

    async fn upload_avatars(
        &self,
        request: Request<UploadAvatarsRequest>,
    ) -> Result<Response<UploadAvatarsReply>, Status> {
        let req = request.into_inner();
        if self.lookup_user(&req.user_id).await.is_none() {
            return Err(Status::not_found(format!("user {} not found", req.user_id)));
        }

        let mut sizes = Vec::with_capacity(req.avatars.len());
        for (idx, blob) in req.avatars.iter().enumerate() {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "greeter_avatar_{}_{}.bin",
                safe_filename(&req.user_id),
                idx
            ));
            fs::write(&path, blob)
                .await
                .map_err(|e| Status::internal(format!("failed to write avatar: {e}")))?;
            info!(
                "stored avatar {} for {} at {}",
                idx,
                req.user_id,
                path.display()
            );
            sizes.push(blob.len() as u64);
        }

        let reply = UploadAvatarsReply {
            user_id: req.user_id,
            sizes,
        };
        Ok(Response::new(reply))
    }
}

impl ExampleGreeter {
    async fn build_reply(&self, name: String, suffix: &str) -> HelloReply {
        let greeting = self.greeting.read().await.clone();
        let user = self.lookup_user("demo").await;

        HelloReply {
            message: format!("{greeting}, {name}!"),
            meta: Some(GreetMeta {
                correlation_id: format!("hello-{suffix}-{name}"),
                from: user,
            }),
        }
    }

    async fn lookup_user(&self, id: &str) -> Option<User> {
        self.users.read().await.get(id).cloned()
    }
}

fn normalize_name(name: String) -> String {
    if name.trim().is_empty() {
        "World".to_string()
    } else {
        name
    }
}

fn safe_filename(input: &str) -> String {
    input
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
