fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Rebuild if proto or script changes
    println!("cargo:rerun-if-changed=proto/graphql.proto");
    println!("cargo:rerun-if-changed=proto/greeter.proto");
    println!("cargo:rerun-if-changed=proto/federation_example.proto");
    println!("cargo:rerun-if-changed=build.rs");

    // Use src/generated directory for generated files
    let generated_dir = std::path::PathBuf::from("src/generated");
    std::fs::create_dir_all(&generated_dir)?;

    // Path to google/protobuf/*.proto (provided by prost)
    let proto_include =
        std::env::var("PROTOC_INCLUDE").unwrap_or_else(|_| "/usr/local/include".to_string());
    let proto_paths = ["proto", &proto_include];

    // Build graphql.proto
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .out_dir(&generated_dir)
        .file_descriptor_set_path(generated_dir.join("graphql_descriptor.bin"))
        .compile_protos(&["proto/graphql.proto"], &proto_paths)?;

    // Build the greeter example descriptor + generated code for the example binary
    tonic_build::configure()
        .out_dir(&generated_dir)
        .file_descriptor_set_path(generated_dir.join("greeter_descriptor.bin"))
        .compile_protos(&["proto/greeter.proto"], &proto_paths)?;

    // Build federation example (optional, for demonstration)
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .out_dir(&generated_dir)
        .file_descriptor_set_path(generated_dir.join("federation_example_descriptor.bin"))
        .compile_protos(&["proto/federation_example.proto"], &proto_paths)?;

    Ok(())
}
