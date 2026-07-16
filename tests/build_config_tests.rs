#[test]
fn encrypted_dns_resolver_is_built_with_trusted_root_certificates() {
    let manifest: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let features = manifest["dependencies"]["hickory-resolver"]["features"]
        .as_array()
        .expect("hickory-resolver features must be explicitly configured");

    assert!(
        features
            .iter()
            .any(|feature| { matches!(feature.as_str(), Some("webpki-roots" | "native-certs")) }),
        "DoH/DoT requires hickory-resolver to be built with a trusted root certificate source"
    );
}
