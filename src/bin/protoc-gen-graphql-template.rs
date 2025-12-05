//! protoc plugin that emits a starter GraphQL gateway for the services in the
//! provided `.proto` files. The output is a single `graphql_gateway.rs` file
//! that you can drop into your project and fill in service endpoints.

use grpc_graphql_gateway::graphql::{GraphqlSchema, GraphqlService, GraphqlType};
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage, ExtensionDescriptor, Value};
use prost_types::compiler::{code_generator_response, CodeGeneratorResponse};

use std::collections::HashSet;
use std::io::{Read, Write};

/// Collected service metadata for template generation.
#[derive(Debug, Clone)]
struct ServiceInfo {
    /// Fully-qualified service name (e.g. package.Service)
    full_name: String,
    /// Optional endpoint from `(graphql.service)` option
    endpoint: Option<String>,
    /// Whether to connect insecurely (defaults to true if not provided)
    insecure: bool,
    ops: OperationBuckets,
}

#[derive(Debug, Default, Clone)]
struct OperationBuckets {
    queries: Vec<String>,
    mutations: Vec<String>,
    subscriptions: Vec<String>,
    resolvers: Vec<String>,
}

#[derive(Default)]
struct TemplateOptions {
    descriptor_path: Option<String>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct RawCodeGeneratorRequest {
    #[prost(string, repeated, tag = "1")]
    pub file_to_generate: ::prost::alloc::vec::Vec<String>,
    #[prost(string, optional, tag = "2")]
    pub parameter: Option<String>,
    #[prost(bytes, repeated, tag = "15")]
    pub proto_file: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct RawFileDescriptorSet {
    #[prost(bytes, repeated, tag = "1")]
    pub file: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read CodeGeneratorRequest from stdin
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    let request = RawCodeGeneratorRequest::decode(&*input)?;

    let options = parse_options(request.parameter.as_deref());
    let pool = build_descriptor_pool(&request)?;
    let services = collect_services(&pool, &request)?;
    let content = render_template(&services, &request.file_to_generate, &options);

    let response = CodeGeneratorResponse {
        file: vec![code_generator_response::File {
            name: Some("graphql_gateway.rs".to_string()),
            insertion_point: None,
            content: Some(content),
            generated_code_info: None,
        }],
        ..Default::default()
    };

    let mut output = Vec::new();
    response.encode(&mut output)?;
    std::io::stdout().write_all(&output)?;
    Ok(())
}

fn parse_options(param: Option<&str>) -> TemplateOptions {
    let mut opts = TemplateOptions::default();
    let Some(param) = param else {
        return opts;
    };

    for part in param.split(',').map(|p| p.trim()).filter(|p| !p.is_empty()) {
        if let Some(rest) = part.strip_prefix("descriptor_path=") {
            opts.descriptor_path = Some(rest.to_string());
        }
    }

    opts
}

fn build_descriptor_pool(
    request: &RawCodeGeneratorRequest,
) -> Result<DescriptorPool, Box<dyn std::error::Error>> {
    let fds = RawFileDescriptorSet {
        file: request.proto_file.clone(),
    };
    let mut bytes = Vec::new();
    fds.encode(&mut bytes)?;
    DescriptorPool::decode(bytes.as_slice()).map_err(|e| e.into())
}

/// Collect the services from the files protoc asked us to generate for, along with
/// GraphQL operations (queries, mutations, subscriptions, resolvers).
fn collect_services(
    pool: &DescriptorPool,
    request: &RawCodeGeneratorRequest,
) -> Result<Vec<ServiceInfo>, Box<dyn std::error::Error>> {
    let targets: HashSet<&str> = request
        .file_to_generate
        .iter()
        .map(|s| s.as_str())
        .collect();

    let method_ext = pool.get_extension_by_name("graphql.schema");
    let service_ext = pool.get_extension_by_name("graphql.service");

    let mut services = Vec::new();
    for svc in pool.services() {
        if !targets.contains(svc.parent_file().name()) {
            continue;
        }

        let mut info = ServiceInfo {
            full_name: svc.full_name().to_string(),
            endpoint: None,
            insecure: true,
            ops: OperationBuckets::default(),
        };

        if let Some(ext) = service_ext.as_ref() {
            if let Some(opts) = decode_extension::<GraphqlService>(&svc.options(), ext)? {
                if !opts.host.is_empty() {
                    info.endpoint = Some(opts.host);
                }
                info.insecure = opts.insecure;
            }
        }

        if let Some(method_ext) = method_ext.as_ref() {
            for method in svc.methods() {
                let Some(schema_opts) =
                    decode_extension::<GraphqlSchema>(&method.options(), method_ext)?
                else {
                    continue;
                };

                let graphql_name = if schema_opts.name.is_empty() {
                    method.name().to_string()
                } else {
                    schema_opts.name.clone()
                };

                match GraphqlType::try_from(schema_opts.r#type).unwrap_or(GraphqlType::Query) {
                    GraphqlType::Query => info.ops.queries.push(graphql_name),
                    GraphqlType::Mutation => info.ops.mutations.push(graphql_name),
                    GraphqlType::Subscription => info.ops.subscriptions.push(graphql_name),
                    GraphqlType::Resolver => info.ops.resolvers.push(graphql_name),
                }
            }
        }

        services.push(info);
    }

    services.sort_by(|a, b| a.full_name.cmp(&b.full_name));
    Ok(services)
}

fn decode_extension<T: Message + Default>(
    opts: &DynamicMessage,
    ext: &ExtensionDescriptor,
) -> Result<Option<T>, Box<dyn std::error::Error>> {
    if !opts.has_extension(ext) {
        return Ok(None);
    }

    let val = opts.get_extension(ext);
    if let Value::Message(msg) = val.as_ref() {
        return T::decode(msg.encode_to_vec().as_slice())
            .map(Some)
            .map_err(|e| e.into());
    }

    Ok(None)
}



/// Render the Rust template that wires the gateway together.
fn render_template(
    services: &[ServiceInfo],
    files: &[String],
    options: &TemplateOptions,
) -> String {
    let all_queries = collect_ops(services, |ops| &ops.queries);
    let all_mutations = collect_ops(services, |ops| &ops.mutations);
    let all_subscriptions = collect_ops(services, |ops| &ops.subscriptions);

    let mut buf = String::new();
    buf.push_str("// @generated by protoc-gen-graphql-template\n");
    buf.push_str("// Source files:\n");
    for file in files {
        buf.push_str(&format!("//   - {file}\n"));
    }
    buf.push_str("//\n");
    buf.push_str("// This is a starter gateway. Update endpoint URLs and tweak as needed.\n\n");

    buf.push_str("use grpc_graphql_gateway::{Gateway, GatewayBuilder, GrpcClient, Result};\n");
    buf.push_str("use tracing_subscriber::prelude::*;\n\n");
    let descriptor_expr = options
        .descriptor_path
        .as_ref()
        .map(|p| format!("\"{}\"", p.escape_default()))
        .unwrap_or_else(|| "concat!(env!(\"OUT_DIR\"), \"/graphql_descriptor.bin\")".to_string());
    buf.push_str(&format!(
        "const DESCRIPTOR_SET: &[u8] = include_bytes!({descriptor_expr});\n\n"
    ));

    buf.push_str("fn describe(list: &[&str]) -> String {\n");
    buf.push_str("    if list.is_empty() { \"none\".to_string() } else { list.join(\", \") }\n");
    buf.push_str("}\n\n");

    buf.push_str(&format!(
        "const QUERIES: &[&str] = {};\n",
        render_str_slice(&all_queries)
    ));
    buf.push_str(&format!(
        "const MUTATIONS: &[&str] = {};\n",
        render_str_slice(&all_mutations)
    ));
    buf.push_str(&format!(
        "const SUBSCRIPTIONS: &[&str] = {};\n\n",
        render_str_slice(&all_subscriptions)
    ));

    buf.push_str("struct ServiceConfig {\n");
    buf.push_str("    name: &'static str,\n");
    buf.push_str("    endpoint: &'static str,\n");
    buf.push_str("    insecure: bool,\n");
    buf.push_str("    queries: &'static [&'static str],\n");
    buf.push_str("    mutations: &'static [&'static str],\n");
    buf.push_str("    subscriptions: &'static [&'static str],\n");
    buf.push_str("    resolvers: &'static [&'static str],\n");
    buf.push_str("}\n\n");

    buf.push_str("const SERVICES: &[ServiceConfig] = &[\n");
    for svc in services {
        buf.push_str("    ServiceConfig {\n");
        buf.push_str(&format!(
            "        name: {},\n",
            render_str_literal(&svc.full_name)
        ));
        let endpoint = svc.endpoint.as_deref().unwrap_or("http://127.0.0.1:50051");
        buf.push_str(&format!(
            "        endpoint: {},\n",
            render_str_literal(endpoint)
        ));
        buf.push_str(&format!("        insecure: {},\n", svc.insecure));
        buf.push_str(&format!(
            "        queries: {},\n",
            render_str_slice(&svc.ops.queries)
        ));
        buf.push_str(&format!(
            "        mutations: {},\n",
            render_str_slice(&svc.ops.mutations)
        ));
        buf.push_str(&format!(
            "        subscriptions: {},\n",
            render_str_slice(&svc.ops.subscriptions)
        ));
        buf.push_str(&format!(
            "        resolvers: {},\n",
            render_str_slice(&svc.ops.resolvers)
        ));
        buf.push_str("    },\n");
    }
    buf.push_str("];\n\n");

    buf.push_str("pub fn gateway_builder() -> Result<GatewayBuilder> {\n");
    buf.push_str("    // The descriptor set is produced by your build.rs using tonic-build.\n");
    buf.push_str("    let mut builder = Gateway::builder()\n");
    buf.push_str("        .with_descriptor_set_bytes(DESCRIPTOR_SET);\n\n");

    if services.is_empty() {
        buf.push_str("    // TODO: add gRPC clients. Example:\n");
        buf.push_str("    // builder = builder.add_grpc_client(\n");
        buf.push_str("    //     \"my.package.Service\",\n");
        buf.push_str("    //     GrpcClient::connect_lazy(\"http://127.0.0.1:50051\", true)?,\n");
        buf.push_str("    // );\n");
    } else {
        buf.push_str("    // Add gRPC backends for each service discovered in your protos.\n");
        buf.push_str("    let mut clients = Vec::new();\n");
        buf.push_str("    for svc in SERVICES {\n");
        buf.push_str("        tracing::info!(\n");
        buf.push_str("            \"{svc} -> {endpoint} (queries: {queries}; mutations: {mutations}; subscriptions: {subscriptions}; resolvers: {resolvers})\",\n");
        buf.push_str("            svc = svc.name,\n");
        buf.push_str("            endpoint = svc.endpoint,\n");
        buf.push_str("            queries = describe(svc.queries),\n");
        buf.push_str("            mutations = describe(svc.mutations),\n");
        buf.push_str("            subscriptions = describe(svc.subscriptions),\n");
        buf.push_str("            resolvers = describe(svc.resolvers),\n");
        buf.push_str("        );\n");
        buf.push_str("        clients.push((\n");
        buf.push_str("            svc.name.to_string(),\n");
        buf.push_str("            GrpcClient::builder(svc.endpoint)\n");
        buf.push_str("                .insecure(svc.insecure)\n");
        buf.push_str("                .lazy(true)\n");
        buf.push_str("                .connect_lazy()?,\n");
        buf.push_str("        ));\n");
        buf.push_str("    }\n");
        buf.push_str("    builder = builder.add_grpc_clients(clients);\n");
        buf.push_str("\n    // Update the endpoints above to point at your actual services.\n");
    }

    buf.push_str("\n    Ok(builder)\n}\n\n");

    buf.push_str("#[tokio::main]\n");
    buf.push_str("async fn main() -> Result<()> {\n");
    buf.push_str("    // Basic logging; adjust as desired.\n");
    buf.push_str("    tracing_subscriber::registry()\n");
    buf.push_str("        .with(tracing_subscriber::fmt::layer())\n");
    buf.push_str("        .init();\n\n");
    buf.push_str("    tracing::info!(\n");
    buf.push_str("        \"GraphQL operations -> queries: {queries}; mutations: {mutations}; subscriptions: {subscriptions}\",\n");
    buf.push_str("        queries = describe(QUERIES),\n");
    buf.push_str("        mutations = describe(MUTATIONS),\n");
    buf.push_str("        subscriptions = describe(SUBSCRIPTIONS),\n");
    buf.push_str("    );\n\n");
    buf.push_str("    // NOTE: Resolver entries are listed above; the runtime currently warns that they are not implemented.\n");
    buf.push_str("    gateway_builder()?\n");
    buf.push_str("        .serve(\"0.0.0.0:8888\")\n");
    buf.push_str("        .await\n");
    buf.push_str("}\n");

    buf
}

fn render_str_literal(input: &str) -> String {
    format!("\"{}\"", input.escape_default())
}

fn render_str_slice(values: &[String]) -> String {
    if values.is_empty() {
        "&[]".to_string()
    } else {
        let joined = values
            .iter()
            .map(|v| render_str_literal(v))
            .collect::<Vec<_>>()
            .join(", ");
        format!("&[{joined}]")
    }
}

fn collect_ops<F>(services: &[ServiceInfo], f: F) -> Vec<String>
where
    F: Fn(&OperationBuckets) -> &Vec<String>,
{
    let mut set = std::collections::BTreeSet::new();
    for svc in services {
        for op in f(&svc.ops) {
            set.insert(op.clone());
        }
    }
    set.into_iter().collect()
}
