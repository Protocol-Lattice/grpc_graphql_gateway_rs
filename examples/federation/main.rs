use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use async_graphql::{Name, Value as GqlValue};
use grpc_graphql_gateway::{EntityResolver, Gateway, GrpcClient};
use tokio::sync::RwLock;
use tonic::{transport::Server, Request, Response, Status};
use tracing::info;

pub mod federation {
    include!("../../src/generated/federation_example.rs");
}

use federation::product_service_server::{ProductService, ProductServiceServer};
use federation::review_service_server::{ReviewService, ReviewServiceServer};
use federation::user_service_server::{UserService, UserServiceServer};
use federation::{
    GetProductRequest, GetProductResponse, GetReviewRequest, GetReviewResponse, GetUserRequest,
    GetUserResponse, GetUserReviewsRequest, GetUserReviewsResponse, Product, Review, User,
};

const DESCRIPTORS: &[u8] = include_bytes!("../../src/generated/federation_example_descriptor.bin");
const USER_GRPC_ADDR: &str = "127.0.0.1:50051";
const PRODUCT_GRPC_ADDR: &str = "127.0.0.1:50052";
const REVIEW_GRPC_ADDR: &str = "127.0.0.1:50053";
const USER_GRAPH_ADDR: &str = "127.0.0.1:8891";
const PRODUCT_GRAPH_ADDR: &str = "127.0.0.1:8892";
const REVIEW_GRAPH_ADDR: &str = "127.0.0.1:8893";
const ROUTER_ADDR: &str = "127.0.0.1:4000";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .compact()
        .init();

    let store = Arc::new(RwLock::new(FederationData::seed()));
    let services = FederationServices::with_store(store.clone());
    let entity_resolver = Arc::new(ExampleEntityResolver::new(store));

    print_examples();

    tokio::try_join!(
        run_user_service(services.clone()),
        run_product_service(services.clone()),
        run_review_service(services.clone()),
        run_user_gateway(entity_resolver.clone()),
        run_product_gateway(entity_resolver.clone()),
        run_review_gateway(entity_resolver),
    )?;

    Ok(())
}

async fn run_user_service(services: FederationServices) -> Result<()> {
    let addr: SocketAddr = USER_GRPC_ADDR.parse()?;
    info!("UserService listening on {}", addr);

    Server::builder()
        .add_service(UserServiceServer::new(services))
        .serve(addr)
        .await?;

    Ok(())
}

async fn run_product_service(services: FederationServices) -> Result<()> {
    let addr: SocketAddr = PRODUCT_GRPC_ADDR.parse()?;
    info!("ProductService listening on {}", addr);

    Server::builder()
        .add_service(ProductServiceServer::new(services))
        .serve(addr)
        .await?;

    Ok(())
}

async fn run_review_service(services: FederationServices) -> Result<()> {
    let addr: SocketAddr = REVIEW_GRPC_ADDR.parse()?;
    info!("ReviewService listening on {}", addr);

    Server::builder()
        .add_service(ReviewServiceServer::new(services))
        .serve(addr)
        .await?;

    Ok(())
}

async fn run_user_gateway(entity_resolver: Arc<dyn EntityResolver>) -> Result<()> {
    let user_client = GrpcClient::builder(format!("http://{USER_GRPC_ADDR}")).connect_lazy()?;

    info!(
        "User subgraph listening on http://{}/graphql",
        USER_GRAPH_ADDR
    );

    Gateway::builder()
        .with_descriptor_set_bytes(DESCRIPTORS)
        .enable_federation()
        .with_entity_resolver(entity_resolver)
        .add_grpc_clients([("federation_example.UserService".to_string(), user_client)])
        .with_services(["federation_example.UserService"])
        .serve(USER_GRAPH_ADDR)
        .await?;

    Ok(())
}

async fn run_product_gateway(entity_resolver: Arc<dyn EntityResolver>) -> Result<()> {
    let product_client =
        GrpcClient::builder(format!("http://{PRODUCT_GRPC_ADDR}")).connect_lazy()?;

    info!(
        "Product subgraph listening on http://{}/graphql",
        PRODUCT_GRAPH_ADDR
    );

    Gateway::builder()
        .with_descriptor_set_bytes(DESCRIPTORS)
        .enable_federation()
        .with_entity_resolver(entity_resolver)
        .add_grpc_clients([(
            "federation_example.ProductService".to_string(),
            product_client,
        )])
        .with_services(["federation_example.ProductService"])
        .serve(PRODUCT_GRAPH_ADDR)
        .await?;

    Ok(())
}

async fn run_review_gateway(entity_resolver: Arc<dyn EntityResolver>) -> Result<()> {
    let review_client = GrpcClient::builder(format!("http://{REVIEW_GRPC_ADDR}")).connect_lazy()?;

    info!(
        "Review subgraph listening on http://{}/graphql",
        REVIEW_GRAPH_ADDR
    );

    Gateway::builder()
        .with_descriptor_set_bytes(DESCRIPTORS)
        .enable_federation()
        .with_entity_resolver(entity_resolver)
        .add_grpc_clients([(
            "federation_example.ReviewService".to_string(),
            review_client,
        )])
        .with_services(["federation_example.ReviewService"])
        .serve(REVIEW_GRAPH_ADDR)
        .await?;

    Ok(())
}

#[derive(Clone, Default)]
struct FederationServices {
    store: Arc<RwLock<FederationData>>,
}

impl FederationServices {
    fn new(data: FederationData) -> Self {
        Self {
            store: Arc::new(RwLock::new(data)),
        }
    }

    fn with_store(store: Arc<RwLock<FederationData>>) -> Self {
        Self { store }
    }
}

#[tonic::async_trait]
impl UserService for FederationServices {
    async fn get_user(
        &self,
        request: Request<GetUserRequest>,
    ) -> Result<Response<GetUserResponse>, Status> {
        let id = request.into_inner().id;
        let user = self.store.read().await.users.get(&id).cloned();

        Ok(Response::new(GetUserResponse { user }))
    }
}

#[tonic::async_trait]
impl ProductService for FederationServices {
    async fn get_product(
        &self,
        request: Request<GetProductRequest>,
    ) -> Result<Response<GetProductResponse>, Status> {
        let upc = request.into_inner().upc;
        let product = self.store.read().await.products.get(&upc).cloned();

        Ok(Response::new(GetProductResponse { product }))
    }
}

#[tonic::async_trait]
impl ReviewService for FederationServices {
    async fn get_review(
        &self,
        request: Request<GetReviewRequest>,
    ) -> Result<Response<GetReviewResponse>, Status> {
        let id = request.into_inner().id;
        let review = self.store.read().await.reviews.get(&id).cloned();

        Ok(Response::new(GetReviewResponse { review }))
    }

    async fn get_user_reviews(
        &self,
        request: Request<GetUserReviewsRequest>,
    ) -> Result<Response<GetUserReviewsResponse>, Status> {
        let user_id = request.into_inner().user_id;
        let data = self.store.read().await;
        let reviews = data
            .reviews
            .values()
            .filter(|review| {
                review
                    .author
                    .as_ref()
                    .map(|user| user.id == user_id)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        Ok(Response::new(GetUserReviewsResponse { reviews }))
    }
}

#[derive(Clone, Default)]
struct FederationData {
    users: HashMap<String, User>,
    products: HashMap<String, Product>,
    reviews: HashMap<String, Review>,
}

impl FederationData {
    fn seed() -> Self {
        let mut users = HashMap::new();
        let mut products = HashMap::new();
        let mut reviews = HashMap::new();

        let alice = User {
            id: "u1".to_string(),
            email: "alice@example.com".to_string(),
            name: "Alice".to_string(),
        };
        let bob = User {
            id: "u2".to_string(),
            email: "bob@example.com".to_string(),
            name: "Bob".to_string(),
        };
        users.insert(alice.id.clone(), alice.clone());
        users.insert(bob.id.clone(), bob.clone());

        let rocket = Product {
            upc: "apollo-1".to_string(),
            name: "Apollo Rocket".to_string(),
            price: 499,
            created_by: Some(alice.clone()),
        };
        let satchel = Product {
            upc: "astro-42".to_string(),
            name: "Astro Satchel".to_string(),
            price: 149,
            created_by: Some(bob.clone()),
        };
        products.insert(rocket.upc.clone(), rocket.clone());
        products.insert(satchel.upc.clone(), satchel.clone());

        reviews.insert(
            "r1".to_string(),
            Review {
                id: "r1".to_string(),
                product: Some(rocket),
                author: Some(bob.clone()),
                body: "Launches straight and true.".to_string(),
                rating: 5,
            },
        );
        reviews.insert(
            "r2".to_string(),
            Review {
                id: "r2".to_string(),
                product: Some(satchel),
                author: Some(alice),
                body: "Fits every mission checklist.".to_string(),
                rating: 4,
            },
        );

        Self {
            users,
            products,
            reviews,
        }
    }
}

fn print_examples() {
    println!("User subgraph:    http://{}/graphql", USER_GRAPH_ADDR);
    println!("Product subgraph: http://{}/graphql", PRODUCT_GRAPH_ADDR);
    println!("Review subgraph:  http://{}/graphql", REVIEW_GRAPH_ADDR);
    println!(
        "Apollo Router (after composing supergraph) default: http://{}/",
        ROUTER_ADDR
    );
    println!("Try these once the services (and router) are up:");
    println!("  query {{ user(id:\"u1\") {{ id email name }} }}");
    println!(
        "  query {{ product(upc:\"apollo-1\") {{ upc name price createdBy {{ id name }} }} }}"
    );
    println!("  query {{ userReviews(userId:\"u1\") {{ id rating body author {{ id name }} product {{ upc name }} }} }}");
    println!(
        "  query {{ _entities(representations:[{{ __typename:\"federation_example_User\", id:\"u1\" }}]) {{ ... on federation_example_User {{ id email name }} }} }}"
    );
}

#[derive(Clone)]
struct ExampleEntityResolver {
    store: Arc<RwLock<FederationData>>,
}

impl ExampleEntityResolver {
    fn new(store: Arc<RwLock<FederationData>>) -> Self {
        Self { store }
    }

    fn user_to_value(user: &User) -> GqlValue {
        let mut obj = async_graphql::indexmap::IndexMap::new();
        obj.insert(Name::new("id"), GqlValue::String(user.id.clone()));
        obj.insert(Name::new("email"), GqlValue::String(user.email.clone()));
        obj.insert(Name::new("name"), GqlValue::String(user.name.clone()));
        GqlValue::Object(obj)
    }

    fn product_to_value(product: &Product) -> GqlValue {
        let mut obj = async_graphql::indexmap::IndexMap::new();
        obj.insert(Name::new("upc"), GqlValue::String(product.upc.clone()));
        obj.insert(Name::new("name"), GqlValue::String(product.name.clone()));
        obj.insert(Name::new("price"), GqlValue::Number(product.price.into()));

        if let Some(author) = product.created_by.as_ref() {
            obj.insert(Name::new("createdBy"), Self::user_to_value(author));
        }

        GqlValue::Object(obj)
    }

    fn review_to_value(review: &Review) -> GqlValue {
        let mut obj = async_graphql::indexmap::IndexMap::new();
        obj.insert(Name::new("id"), GqlValue::String(review.id.clone()));
        obj.insert(Name::new("body"), GqlValue::String(review.body.clone()));
        obj.insert(Name::new("rating"), GqlValue::Number(review.rating.into()));

        if let Some(prod) = review.product.as_ref() {
            obj.insert(Name::new("product"), Self::product_to_value(prod));
        }
        if let Some(author) = review.author.as_ref() {
            obj.insert(Name::new("author"), Self::user_to_value(author));
        }

        GqlValue::Object(obj)
    }

    fn required_str<'a>(
        representation: &'a async_graphql::indexmap::IndexMap<Name, GqlValue>,
        key: &str,
    ) -> grpc_graphql_gateway::Result<&'a str> {
        representation
            .get(&Name::new(key))
            .and_then(|v| match v {
                GqlValue::String(s) => Some(s.as_str()),
                _ => None,
            })
            .ok_or_else(|| grpc_graphql_gateway::Error::Schema(format!("missing key {key}")))
    }
}

#[async_trait::async_trait]
impl EntityResolver for ExampleEntityResolver {
    async fn resolve_entity(
        &self,
        entity_config: &grpc_graphql_gateway::federation::EntityConfig,
        representation: &async_graphql::indexmap::IndexMap<Name, GqlValue>,
    ) -> grpc_graphql_gateway::Result<GqlValue> {
        let data = self.store.read().await;
        match entity_config.type_name.as_str() {
            "federation_example_User" => {
                let id = Self::required_str(representation, "id")?;
                let user = data.users.get(id).ok_or_else(|| {
                    grpc_graphql_gateway::Error::Schema(format!("user {id} not found"))
                })?;
                Ok(Self::user_to_value(user))
            }
            "federation_example_Product" => {
                let upc = Self::required_str(representation, "upc")?;
                let product = data.products.get(upc).ok_or_else(|| {
                    grpc_graphql_gateway::Error::Schema(format!("product {upc} not found"))
                })?;
                Ok(Self::product_to_value(product))
            }
            "federation_example_Review" => {
                let id = Self::required_str(representation, "id")?;
                let review = data.reviews.get(id).ok_or_else(|| {
                    grpc_graphql_gateway::Error::Schema(format!("review {id} not found"))
                })?;
                Ok(Self::review_to_value(review))
            }
            other => Err(grpc_graphql_gateway::Error::Schema(format!(
                "unknown entity type: {other}"
            ))),
        }
    }
}
