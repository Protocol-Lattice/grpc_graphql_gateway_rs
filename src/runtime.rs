//! Runtime support for GraphQL gateway - HTTP and WebSocket integration.

use crate::error::{GraphQLError, Result};
use crate::middleware::{Context, Middleware};
use crate::schema::{DynamicSchema, GrpcResponseCache};
use async_graphql::ServerError;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse, GraphQLSubscription};
use axum::{
    extract::State,
    http::HeaderMap,
    response::{Html, IntoResponse},
    routing::{get_service, post},
    Extension, Router,
};
use std::sync::Arc;

/// ServeMux - main gateway handler
pub struct ServeMux {
    schema: DynamicSchema,
    middlewares: Vec<Arc<dyn Middleware>>,
    error_handler: Option<Arc<dyn Fn(Vec<GraphQLError>) + Send + Sync>>,
}

impl ServeMux {
    /// Create a new ServeMux with an already built schema
    pub fn new(schema: DynamicSchema) -> Self {
        Self {
            schema,
            middlewares: Vec::new(),
            error_handler: None,
        }
    }

    /// Add middleware
    pub fn add_middleware(&mut self, middleware: Arc<dyn Middleware>) {
        self.middlewares.push(middleware);
    }

    /// Use middleware (builder pattern)
    pub fn with_middleware(mut self, middleware: Arc<dyn Middleware>) -> Self {
        self.add_middleware(middleware);
        self
    }

    /// Set error handler from an `Arc` for cases where the caller already shares ownership.
    pub fn set_error_handler_arc(&mut self, handler: Arc<dyn Fn(Vec<GraphQLError>) + Send + Sync>) {
        self.error_handler = Some(handler);
    }

    /// Set error handler
    pub fn set_error_handler<F>(&mut self, handler: F)
    where
        F: Fn(Vec<GraphQLError>) + Send + Sync + 'static,
    {
        self.set_error_handler_arc(Arc::new(handler));
    }

    async fn execute_with_middlewares(
        &self,
        headers: HeaderMap,
        request: GraphQLRequest,
    ) -> Result<async_graphql::Response> {
        let mut ctx = Context {
            headers: headers.clone(),
            extensions: std::collections::HashMap::new(),
        };

        for middleware in &self.middlewares {
            middleware.call(&mut ctx).await?;
        }

        let mut gql_request = request.into_inner();
        gql_request = gql_request.data(ctx);
        gql_request = gql_request.data(GrpcResponseCache::default());

        Ok(self.schema.execute(gql_request).await)
    }

    /// Handle GraphQL HTTP request
    pub async fn handle_http(
        &self,
        headers: HeaderMap,
        request: GraphQLRequest,
    ) -> GraphQLResponse {
        match self.execute_with_middlewares(headers, request).await {
            Ok(resp) => resp.into(),
            Err(err) => {
                let gql_err: GraphQLError = err.into();
                if let Some(handler) = &self.error_handler {
                    handler(vec![gql_err.clone()]);
                }
                let server_err = ServerError::new(gql_err.message.clone(), None);
                async_graphql::Response::from_errors(vec![server_err]).into()
            }
        }
    }

    /// Convert to Axum router
    pub fn into_router(self) -> Router {
        let state = Arc::new(self);
        let subscription = GraphQLSubscription::new(state.schema.executor());

        Router::new()
            .route(
                "/graphql",
                post(handle_graphql_post).get(graphql_playground),
            )
            .route_service("/graphql/ws", get_service(subscription))
            .layer(Extension(state.schema.executor()))
            .with_state(state)
    }
}

impl Clone for ServeMux {
    fn clone(&self) -> Self {
        Self {
            schema: self.schema.clone(),
            middlewares: self.middlewares.clone(),
            error_handler: self.error_handler.clone(),
        }
    }
}

/// Handler for POST requests to /graphql
async fn handle_graphql_post(
    State(mux): State<Arc<ServeMux>>,
    headers: HeaderMap,
    request: GraphQLRequest,
) -> impl IntoResponse {
    mux.handle_http(headers, request).await
}

/// Serve the GraphQL Playground UI for ad-hoc exploration.
async fn graphql_playground() -> impl IntoResponse {
    Html(async_graphql::http::playground_source(
        async_graphql::http::GraphQLPlaygroundConfig::new("/graphql")
            .subscription_endpoint("/graphql/ws"),
    ))
}
