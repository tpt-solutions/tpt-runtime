//! Archon storage adapter (SPEC §16): a runtime volume hosts a fixed-size
//! Archon [`BlockDevice`], so an Archon `StorageEngine`/`Database` can live
//! inside a logical volume with WAL-durable pages — the seam where the
//! Archon substrate takes over from plain directories, one volume at a
//! time.
//!
//! Feature-gated (`archon`) so the default build keeps no Archon
//! dependency.

use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;
use tpt_archon_core::block::{BlockDevice, StorageError};
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};

/// Bytes per block; mirrors the Archon default (and ).
const BLOCK_SIZE: usize = 4096;

/// A fixed-capacity block device backed by one file inside a volume's
/// backing directory. All I/O is bounds-checked against `block_count`.
pub struct VolumeBlockDevice {
    file: Mutex<std::fs::File>,
    block_count: u64,
}

/// Upper bound for a single Archon device inside a volume (4 GiB).
pub const MAX_DEVICE_BLOCKS: u64 = (4 * 1024 * 1024 * 1024) / BLOCK_SIZE as u64;

impl VolumeBlockDevice {
    /// Creates (or resizes) the device file `path` to hold `block_count`
    /// blocks. Refuses capacities above [`MAX_DEVICE_BLOCKS`].
    pub fn create(path: &std::path::Path, block_count: u64) -> Result<Self, StorageError> {
        if block_count == 0 || block_count > MAX_DEVICE_BLOCKS {
            return Err(StorageError::OutOfBounds {
                block_id: block_count,
                block_count: MAX_DEVICE_BLOCKS,
            });
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(io_err)?;
        let expected = block_count * BLOCK_SIZE as u64;
        if file.metadata().map_err(io_err)?.len() < expected {
            file.set_len(expected).map_err(io_err)?;
        }
        Ok(Self {
            file: Mutex::new(file),
            block_count,
        })
    }
}

impl BlockDevice for VolumeBlockDevice {
    fn read_block(&self, block_id: u64, buffer: &mut [u8]) -> Result<(), StorageError> {
        if buffer.len() != BLOCK_SIZE {
            return Err(StorageError::ShortRead {
                got: buffer.len(),
                expected: BLOCK_SIZE,
            });
        }
        if block_id >= self.block_count {
            return Err(StorageError::OutOfBounds {
                block_id,
                block_count: self.block_count,
            });
        }
        let mut file = self.file.lock().unwrap();
        file.seek(SeekFrom::Start(block_id * BLOCK_SIZE as u64))
            .map_err(io_err)?;
        file.read_exact(buffer).map_err(io_err)
    }

    fn write_block(&mut self, block_id: u64, data: &[u8]) -> Result<(), StorageError> {
        if data.len() != BLOCK_SIZE {
            return Err(StorageError::ShortWrite {
                got: data.len(),
                expected: BLOCK_SIZE,
            });
        }
        if block_id >= self.block_count {
            return Err(StorageError::OutOfBounds {
                block_id,
                block_count: self.block_count,
            });
        }
        let mut file = self.file.lock().unwrap();
        file.seek(SeekFrom::Start(block_id * BLOCK_SIZE as u64))
            .map_err(io_err)?;
        file.write_all(data).map_err(io_err)
    }

    fn sync(&mut self) -> Result<(), StorageError> {
        self.file.lock().unwrap().sync_all().map_err(io_err)
    }

    fn block_count(&self) -> u64 {
        self.block_count
    }
}

fn io_err(err: std::io::Error) -> StorageError {
    StorageError::Io {
        kind: err.kind() as u8,
    }
}

impl crate::StorageManager {
    /// Provisions an Archon block device of `block_count` blocks inside
    /// `volume`'s backing directory, under `file_name`. The volume must
    /// exist; `file_name` must be a plain name (no separators).
    pub fn open_archon_device(
        &self,
        volume: &str,
        file_name: &str,
        block_count: u64,
    ) -> Result<VolumeBlockDevice> {
        let backing = self.get(volume)?.backing_path.clone();
        if !valid_device_name(file_name) {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                format!("invalid archon device name '{file_name}'"),
            ));
        }
        if block_count == 0 || block_count > MAX_DEVICE_BLOCKS {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                format!(
                    "archon device size {block_count} blocks out of range (1..={MAX_DEVICE_BLOCKS})"
                ),
            ));
        }
        VolumeBlockDevice::create(&backing.join(file_name), block_count)
            .map_err(|err| RuntimeError::new(ErrorKind::StorageFailure, format!("{err:?}")))
    }
}

fn valid_device_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_archon_core::page::PAGE_SIZE;
    use tpt_archon_core::storage::StorageEngine;

    fn volume_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tpt-storage-archon-{tag}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn volume_backed_engine_survives_reopen() {
        let base = volume_dir("persist");
        let mut manager = crate::StorageManager::open(&base).unwrap();
        manager.create("dbvol").unwrap();

        // Provision through the storage manager (the documented seam).
        let blocks = 16;
        manager
            .open_archon_device("dbvol", "data.archon", blocks)
            .unwrap();
        let device_path = manager
            .get("dbvol")
            .unwrap()
            .backing_path
            .join("data.archon");

        // Write a page through a StorageEngine on the volume device...
        {
            let device = VolumeBlockDevice::create(&device_path, blocks).unwrap();
            let mut engine = StorageEngine::new(device, 8);
            let mut page = [0u8; PAGE_SIZE];
            page[..7].copy_from_slice(b"durable");
            engine.write_page(3, &page).unwrap();
            engine.commit().unwrap();
            // device drops here (file closed)
        }

        // ...and read it back through a fresh engine on the same volume.
        {
            let device = VolumeBlockDevice::create(&device_path, blocks).unwrap();
            let mut engine = StorageEngine::new(device, 8);
            let page = engine.read_page(3).unwrap();
            assert_eq!(&page[..7], b"durable");
        }

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn device_bounds_are_enforced() {
        let base = volume_dir("bounds");
        let mut manager = crate::StorageManager::open(&base).unwrap();
        manager.create("small").unwrap();
        let path = manager.get("small").unwrap().backing_path.join("d.bin");

        let mut device = VolumeBlockDevice::create(&path, 2).unwrap();
        assert_eq!(device.block_count(), 2);

        // In-bounds round trip.
        let mut buffer = vec![0u8; BLOCK_SIZE];
        buffer[..3].copy_from_slice(b"abc");
        device.write_block(1, &buffer).unwrap();
        let mut read_back = vec![0u8; BLOCK_SIZE];
        device.read_block(1, &mut read_back).unwrap();
        assert_eq!(&read_back[..3], b"abc");

        // Out-of-bounds and wrong-size accesses fail with StorageError.
        assert!(matches!(
            device.write_block(2, &buffer),
            Err(StorageError::OutOfBounds { .. })
        ));
        assert!(matches!(
            device.read_block(0, &mut [0u8; 16]),
            Err(StorageError::ShortRead { .. })
        ));

        // Manager provisioning refuses absurd sizes and bad names.
        assert!(manager
            .open_archon_device("small", "d.bin", MAX_DEVICE_BLOCKS + 1)
            .is_err());
        assert!(manager.open_archon_device("small", "../evil", 4).is_err());
        assert!(manager.open_archon_device("ghost", "d.bin", 4).is_err());

        std::fs::remove_dir_all(&base).ok();
    }
}
