//! Integration: registry → store → bundle over a real HTTP connection
//! (SPEC §45 OCI row). The registry is served by a local fake speaking
//! the Docker Registry HTTP API v2, including the anonymous Bearer token
//! flow, so no external network is involved.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tpt_runtime_model::execution::oci_ref::ImageReference;
use tpt_runtime_oci::registry::RegistryClient;
use tpt_runtime_oci::{image, ImageStore, OciBackend, PullPolicy};
use tpt_runtime_process::ExecutionBackend;

const MANIFEST_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const INDEX_TYPE: &str = "application/vnd.oci.image.index.v1+json";
const LAYER_TYPE: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
const CONFIG_TYPE: &str = "application/vnd.oci.image.config.v1+json";
const TOKEN: &str = "test-token";

/// In-memory registry contents and request counters.
#[derive(Default)]
struct RegistryState {
    manifests: BTreeMap<String, (String, Vec<u8>)>,
    blobs: BTreeMap<String, Vec<u8>>,
    counts: BTreeMap<String, usize>,
    require_auth: bool,
    /// When set, blob requests for this digest serve `bytes` instead
    /// (simulating registry corruption).
    corrupt_blobs: BTreeMap<String, Vec<u8>>,
}

impl RegistryState {
    fn requests(&self, needle: &str) -> usize {
        self.counts
            .iter()
            .filter(|(path, _)| path.contains(needle))
            .map(|(_, count)| *count)
            .sum()
    }
}

/// Serves [`RegistryState`] over HTTP/1.1, one request per connection.
struct FakeRegistry {
    addr: std::net::SocketAddr,
    state: Arc<Mutex<RegistryState>>,
}

impl FakeRegistry {
    fn start(state: RegistryState) -> Self {
        let state = Arc::new(Mutex::new(state));
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let shared = state.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                handle_connection(stream, &shared);
            }
        });
        Self { addr, state }
    }

    fn reference(&self, name: &str, tag: &str) -> ImageReference {
        ImageReference::parse(&format!("localhost:{}/{}:{}", self.addr.port(), name, tag)).unwrap()
    }
}

fn handle_connection(mut stream: TcpStream, state: &Arc<Mutex<RegistryState>>) {
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    // Read until the header terminator.
    while let Ok(1) = stream.read(&mut byte) {
        request.push(byte[0]);
        if request.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&request);
    let request_line = text.lines().next().unwrap_or_default().to_owned();
    let path = request_line.split(' ').nth(1).unwrap_or("/").to_owned();
    let authorized = text
        .to_ascii_lowercase()
        .contains(&format!("authorization: bearer {TOKEN}"));

    let (status, content_type, body) = {
        let mut state = state.lock().unwrap();
        *state.counts.entry(path.clone()).or_insert(0) += 1;

        if state.require_auth && !authorized && !path.starts_with("/token") {
            (401, "application/json".to_owned(), b"{}".to_vec())
        } else if path.starts_with("/token") {
            (
                200,
                "application/json".to_owned(),
                br#"{"token":"test-token"}"#.to_vec(),
            )
        } else if let Some(rest) = path.strip_prefix("/v2/") {
            serve_v2(&mut state, rest)
        } else {
            (404, "application/json".to_owned(), b"{}".to_vec())
        }
    };

    let mut response = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if status == 401 {
        response.push_str(&format!(
            "WWW-Authenticate: Bearer realm=\"http://{}/token\",service=\"tpt-test\",scope=\"repository:app:pull\"\r\n",
            stream.local_addr().unwrap()
        ));
    }
    response.push_str("\r\n");
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

fn serve_v2(state: &mut RegistryState, rest: &str) -> (u16, String, Vec<u8>) {
    // /v2/<repo>/manifests/<locator>
    if let Some((repo, locator)) = split_route(rest, "manifests") {
        let _ = repo;
        if let Some((content_type, bytes)) = state.manifests.get(locator) {
            return (200, content_type.clone(), bytes.clone());
        }
        return (404, "application/json".to_owned(), b"{}".to_vec());
    }
    // /v2/<repo>/blobs/<digest>
    if let Some((_repo, digest)) = split_route(rest, "blobs") {
        if let Some(corrupt) = state.corrupt_blobs.get(digest) {
            return (200, "application/octet-stream".to_owned(), corrupt.clone());
        }
        if let Some(bytes) = state.blobs.get(digest) {
            return (200, "application/octet-stream".to_owned(), bytes.clone());
        }
        return (404, "application/json".to_owned(), b"{}".to_vec());
    }
    // /v2/ ping
    (200, "application/json".to_owned(), b"{}".to_vec())
}

fn split_route<'a>(rest: &'a str, kind: &str) -> Option<(&'a str, &'a str)> {
    let needle = format!("/{kind}/");
    let pos = rest.find(&needle)?;
    let repo = &rest[..pos];
    let locator = &rest[pos + needle.len()..];
    Some((repo, locator))
}

/// Builds a gzipped tar layer from (path, contents) entries.
fn layer_bytes(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, contents) in entries {
        let mut header = tar::Header::new_gnu();
        match contents {
            None => {
                header.set_entry_type(tar::EntryType::Directory);
                header.set_size(0);
                header.set_path(path).unwrap();
                header.set_mode(0o755);
                header.set_cksum();
                builder.append(&header, std::io::empty()).unwrap();
            }
            Some(bytes) => {
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                header.set_path(path).unwrap();
                header.set_cksum();
                builder.append(&header, *bytes).unwrap();
            }
        }
    }
    let tar = builder.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&tar).unwrap();
    encoder.finish().unwrap()
}

fn digest_of(bytes: &[u8]) -> String {
    tpt_runtime_oci::digest_of(bytes)
}

/// A small two-layer image: whiteout + app files, PascalCase config.
struct TestImage {
    manifest: Vec<u8>,
    manifest_digest: String,
    config: Vec<u8>,
    config_digest: String,
    layers: Vec<(String, Vec<u8>)>, // (digest, gzipped bytes)
}

fn build_image() -> TestImage {
    let layer1 = layer_bytes(&[
        ("bin", None),
        ("bin/app", Some(b"#!/bin/sh\necho app" as &[u8])),
        ("old.txt", Some(b"stale" as &[u8])),
    ]);
    let layer2 = layer_bytes(&[
        (".wh.old.txt", Some(b"" as &[u8])),
        ("hello.txt", Some(b"hello-from-registry" as &[u8])),
    ]);
    let (layer1_d, layer2_d) = (digest_of(&layer1), digest_of(&layer2));

    let config = br#"{
        "architecture": "amd64",
        "os": "linux",
        "config": {
            "Entrypoint": ["bin/app"],
            "Cmd": ["--serve"],
            "Env": ["PATH=/usr/bin", "APP_MODE=registry-test"],
            "WorkingDir": "/data"
        }
    }"#
    .to_vec();
    let config_digest = digest_of(&config);

    let manifest = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST_TYPE,
        "config": {
            "mediaType": CONFIG_TYPE,
            "digest": config_digest,
            "size": config.len(),
        },
        "layers": [
            {"mediaType": LAYER_TYPE, "digest": layer1_d, "size": layer1.len()},
            {"mediaType": LAYER_TYPE, "digest": layer2_d, "size": layer2.len()},
        ],
    })
    .to_string()
    .into_bytes();
    let manifest_digest = digest_of(&manifest);

    TestImage {
        manifest,
        manifest_digest,
        config,
        config_digest,
        layers: vec![(layer1_d, layer1), (layer2_d, layer2)],
    }
}

fn store() -> (ImageStore, std::path::PathBuf) {
    let base = std::env::temp_dir().join(format!(
        "tpt-oci-reg-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    (ImageStore::open(&base).unwrap(), base)
}

fn registry_with_auth(image: &TestImage) -> FakeRegistry {
    let mut state = RegistryState {
        require_auth: true,
        ..Default::default()
    };
    state.manifests.insert(
        "v1".to_owned(),
        (MANIFEST_TYPE.to_owned(), image.manifest.clone()),
    );
    state.manifests.insert(
        image.manifest_digest.clone(),
        (MANIFEST_TYPE.to_owned(), image.manifest.clone()),
    );
    state
        .blobs
        .insert(image.config_digest.clone(), image.config.clone());
    for (digest, bytes) in &image.layers {
        state.blobs.insert(digest.clone(), bytes.clone());
    }
    FakeRegistry::start(state)
}

#[test]
fn end_to_end_pull_with_auth_and_whiteouts_then_cache() {
    let image = build_image();
    let registry = registry_with_auth(&image);
    let (store, base) = store();

    let reference = registry.reference("app", "v1");
    let client = RegistryClient::insecure(&reference);
    let bundle = image::pull(&client, &store, &reference, 1 << 30).unwrap();

    // Whiteout removed the stale file; both layers' content is present.
    let rootfs = bundle.rootfs_path();
    assert!(!rootfs.join("old.txt").exists(), "whiteout must delete");
    assert_eq!(
        std::fs::read_to_string(rootfs.join("hello.txt")).unwrap(),
        "hello-from-registry"
    );
    assert!(rootfs.join("bin/app").is_file());

    // Run defaults come from the (PascalCase) image config blob.
    assert_eq!(bundle.args, ["bin/app", "--serve"]);
    assert_eq!(
        bundle.env.get("APP_MODE").map(String::as_str),
        Some("registry-test")
    );
    assert_eq!(bundle.working_dir.as_deref(), Some("/data"));
    assert!(
        bundle.path.join("config.json").is_file(),
        "runtime-spec present"
    );

    // Cache: a second pull makes no new registry requests at all.
    let before = {
        let state = registry.state.lock().unwrap();
        state.requests("/blobs/") + state.requests("/manifests/")
    };
    let bundle2 = image::pull(&client, &store, &reference, 1 << 30).unwrap();
    assert_eq!(bundle2.path, bundle.path);
    let after = {
        let state = registry.state.lock().unwrap();
        state.requests("/blobs/") + state.requests("/manifests/")
    };
    assert_eq!(before, after, "second pull must be served from cache");

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn corrupt_layer_blob_is_rejected_by_digest() {
    let image = build_image();
    let mut state = RegistryState::default();
    state.manifests.insert(
        "v1".to_owned(),
        (MANIFEST_TYPE.to_owned(), image.manifest.clone()),
    );
    state.manifests.insert(
        image.manifest_digest.clone(),
        (MANIFEST_TYPE.to_owned(), image.manifest.clone()),
    );
    state
        .blobs
        .insert(image.config_digest.clone(), image.config.clone());
    let (good_digest, _) = &image.layers[1];
    state
        .blobs
        .insert(image.layers[0].0.clone(), image.layers[0].1.clone());
    state
        .corrupt_blobs
        .insert(good_digest.clone(), b"tampered-bytes".to_vec());
    let registry = FakeRegistry::start(state);

    let reference = registry.reference("app", "v1");
    let client = RegistryClient::insecure(&reference);
    let (store, base) = store();

    let err = image::pull(&client, &store, &reference, 1 << 30).unwrap_err();
    assert_eq!(err.kind, tpt_runtime_core::error::ErrorKind::StorageFailure);
    assert!(err.message.contains("digest mismatch"), "{err}");
    // Nothing was tagged: the store stays clean.
    assert!(store.resolve(&reference).is_err());

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn index_selects_the_linux_amd64_manifest() {
    let image = build_image();
    // A windows-amd64 decoy manifest with a marker layer.
    let decoy_layer = layer_bytes(&[("decoy.txt", Some(b"windows" as &[u8]))]);
    let decoy_digest = digest_of(&decoy_layer);
    let decoy_manifest = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST_TYPE,
        "config": {
            "mediaType": CONFIG_TYPE,
            "digest": image.config_digest,
            "size": image.config.len(),
        },
        "layers": [{"mediaType": LAYER_TYPE, "digest": decoy_digest, "size": decoy_layer.len()}],
    })
    .to_string()
    .into_bytes();
    let decoy_manifest_digest = digest_of(&decoy_manifest);

    let index = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": INDEX_TYPE,
        "manifests": [
            {
                "mediaType": MANIFEST_TYPE,
                "digest": decoy_manifest_digest,
                "size": decoy_manifest.len(),
                "platform": {"architecture": "amd64", "os": "windows"},
            },
            {
                "mediaType": MANIFEST_TYPE,
                "digest": image.manifest_digest,
                "size": image.manifest.len(),
                "platform": {"architecture": "amd64", "os": "linux"},
            },
        ],
    })
    .to_string()
    .into_bytes();

    let mut state = RegistryState::default();
    state
        .manifests
        .insert("v2".to_owned(), (INDEX_TYPE.to_owned(), index));
    state.manifests.insert(
        image.manifest_digest.clone(),
        (MANIFEST_TYPE.to_owned(), image.manifest.clone()),
    );
    state.manifests.insert(
        decoy_manifest_digest.clone(),
        (MANIFEST_TYPE.to_owned(), decoy_manifest),
    );
    state
        .blobs
        .insert(image.config_digest.clone(), image.config.clone());
    state.blobs.insert(decoy_digest.clone(), decoy_layer);
    for (digest, bytes) in &image.layers {
        state.blobs.insert(digest.clone(), bytes.clone());
    }
    let registry = FakeRegistry::start(state);

    let reference = registry.reference("app", "v2");
    let client = RegistryClient::insecure(&reference);
    let (store, base) = store();

    let bundle = image::pull(&client, &store, &reference, 1 << 30).unwrap();
    assert!(
        bundle.rootfs_path().join("bin/app").is_file(),
        "the linux/amd64 manifest must be selected, not the decoy"
    );
    assert!(
        !bundle.rootfs_path().join("decoy.txt").exists(),
        "the windows decoy must not be unpacked"
    );

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn backend_prepare_pulls_when_policy_allows() {
    let image = build_image();
    let registry = registry_with_auth(&image);
    let (_store, base) = store();
    let backend = OciBackend::new(base.join("images"))
        .unwrap()
        .with_pull_policy(PullPolicy::IfMissing);
    assert_eq!(backend.pull_policy(), PullPolicy::IfMissing);

    // Point the backend's client at the fake registry by using a
    // loopback reference: prepare_bundle builds an insecure client for
    // loopback hosts.
    let reference = registry.reference("app", "v1");
    let spec = tpt_runtime_model::workload::WorkloadSpec::new(
        "db",
        tpt_runtime_model::execution::ExecutionSpec::OciImage(
            tpt_runtime_model::execution::OciImageSpec {
                image: reference.to_string(),
                ..Default::default()
            },
        ),
    );
    let ctx = tpt_runtime_process::StartContext {
        workload_id: tpt_runtime_core::id::WorkloadId::generate(),
        mounts: vec![],
        log_dir: base.join("logs"),
        capabilities: tpt_runtime_capability::CapabilitySet::empty(),
        network_mode: tpt_runtime_model::network::NetworkMode::None,
        exposed_ports: vec![],
    };
    backend.prepare(&spec, &ctx).unwrap();
    // The image landed in this backend's store.
    assert!(backend.store().resolve(&reference).is_ok());

    std::fs::remove_dir_all(&base).ok();
}
