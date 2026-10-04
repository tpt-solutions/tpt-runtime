//! runtime → Archon integration (SPEC §16, §45): one flow across the
//! adapters — a logical volume provisions an Archon block device, a
//! WAL-durable engine persists pages in it across reopen, and two
//! components exchange bytes through capability-gated shared buffers.
//!
//! Runs with `cargo test -p tpt-runtime-storage --features archon`
//! (and when the workspace is built with the feature enabled).

#![cfg(feature = "archon")]

use tpt_archon_core::page::PAGE_SIZE;
use tpt_archon_core::storage::StorageEngine;
use tpt_runtime_ipc::shared::SharedBufferPool;
use tpt_runtime_storage::archon::VolumeBlockDevice;
use tpt_runtime_storage::StorageManager;

#[test]
fn volume_to_engine_to_shared_buffer_flow() {
    let base = std::env::temp_dir().join(format!(
        "tpt-archon-integration-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let manager = StorageManager::open(&base).unwrap();
    let mut manager = manager;
    manager.create("data").unwrap();

    // 1. The volume provisions the Archon device (the storage seam).
    let device_path = manager
        .get("data")
        .unwrap()
        .backing_path
        .join("store.archon");

    // 2. A durable engine writes two pages into the volume...
    {
        let device = VolumeBlockDevice::create(&device_path, 8).unwrap();
        let mut engine = StorageEngine::new(device, 4);
        let mut first = [0u8; PAGE_SIZE];
        first[..6].copy_from_slice(b"page-0");
        engine.write_page(0, &first).unwrap();
        let mut second = [0u8; PAGE_SIZE];
        second[..6].copy_from_slice(b"page-1");
        engine.write_page(7, &second).unwrap();
        engine.commit().unwrap();
    }

    // 3. ...and a fresh engine over a freshly provisioned device reads
    //    them back: the volume-backed store is durable.
    {
        let device = manager
            .open_archon_device("data", "store.archon", 8)
            .unwrap();
        let mut engine = StorageEngine::new(device, 4);
        assert_eq!(&engine.read_page(0).unwrap()[..6], b"page-0");
        assert_eq!(&engine.read_page(7).unwrap()[..6], b"page-1");
    }

    // 4. Two components share bytes through the capability-gated pool:
    //    the runtime side publishes, the consumer reads in place.
    let mut pool = SharedBufferPool::new(2).unwrap();
    let producer = pool.attach(0).unwrap();
    pool.write(&producer, 0, b"handoff-payload").unwrap();

    let consumer = pool.attach(0).unwrap();
    let seen = pool
        .with_read(&consumer, |page| page[..15].to_vec())
        .unwrap();
    assert_eq!(&seen, b"handoff-payload");

    // 5. Revoking the consumer's capability cuts it off (cache-enforced).
    pool.revoke(&consumer);
    assert!(pool.with_read(&consumer, |_| ()).is_err());

    std::fs::remove_dir_all(&base).ok();
}
