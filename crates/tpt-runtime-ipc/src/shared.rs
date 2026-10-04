//! Archon-backed shared buffers (SPEC §16): fixed-size pages behind the
//! bridge's [`UnifiedPageCache`], access-gated by unforgeable
//! [`Capability`] tokens minted per attached page.
//!
//! Reads borrow the cache's own page in place (zero-copy, via
//! `CapabilityGrant`); writes are the single copy into shared memory. This
//! is the seam where an Archon transport replaces byte-stream copying for
//! logs and stats (Phase 5 of the roadmap).
//!
//! Feature-gated (`archon`) so the default build keeps no Archon
//! dependency.

use std::cell::RefCell;
use std::rc::Rc;
use tpt_archon_bridge::capability::{Capability, CapabilityIssuer, Resource, Right, SharedIssuer};
use tpt_archon_bridge::page_cache::{CacheError, CorePageCache, UnifiedPageCache};
use tpt_archon_core::block::InMemoryBlockDevice;
use tpt_archon_core::page::{BufferPool, Page, PAGE_SIZE};
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};

/// A pool of shared pages, gated by one issuer.
pub struct SharedBufferPool {
    cache: CorePageCache<InMemoryBlockDevice>,
    issuer: SharedIssuer,
    pages: u64,
}

/// One page's access tokens, minted by the pool's issuer.
#[derive(Clone, Copy, Debug)]
pub struct SharedBuffer {
    page: u64,
    read: Capability,
    write: Capability,
}

impl SharedBufferPool {
    /// Creates a pool of `pages` zeroed shared pages.
    pub fn new(pages: u64) -> Result<Self> {
        if pages == 0 || pages > 4096 {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                format!("shared buffer pool size {pages} out of range (1..=4096 pages)"),
            ));
        }
        let issuer: SharedIssuer = Rc::new(RefCell::new(CapabilityIssuer::new()));
        let device = InMemoryBlockDevice::new(pages);
        let pool = BufferPool::new(device, pages as usize);
        Ok(Self {
            cache: CorePageCache::new(pool, issuer.clone()),
            issuer,
            pages,
        })
    }

    /// The page count of this pool.
    pub fn pages(&self) -> u64 {
        self.pages
    }

    /// Mints read/write capabilities for one page.
    pub fn attach(&self, page: u64) -> Result<SharedBuffer> {
        if page >= self.pages {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                format!("page {page} out of range (pool has {} pages)", self.pages),
            ));
        }
        let mut issuer = self.issuer.borrow_mut();
        Ok(SharedBuffer {
            page,
            read: issuer.mint(Resource::Page(page), Right::Read),
            write: issuer.mint(Resource::Page(page), Right::Write),
        })
    }

    /// Writes bytes at `offset` into the shared page. This is the single
    /// copy on the write path.
    pub fn write(&mut self, buffer: &SharedBuffer, offset: usize, bytes: &[u8]) -> Result<usize> {
        if offset + bytes.len() > PAGE_SIZE {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                format!(
                    "write of {} bytes at offset {offset} exceeds the {PAGE_SIZE}-byte page",
                    bytes.len()
                ),
            ));
        }
        let page = self
            .cache
            .map_write(&buffer.write, buffer.page)
            .map_err(cache_err)?;
        page.as_bytes_mut()[offset..offset + bytes.len()].copy_from_slice(bytes);
        self.cache.unmap(buffer.page);
        Ok(bytes.len())
    }

    /// Borrows the shared page in place — no copy — and runs `read`.
    pub fn with_read<R>(
        &mut self,
        buffer: &SharedBuffer,
        read: impl FnOnce(&[u8]) -> R,
    ) -> Result<R> {
        let page: &Page = self
            .cache
            .map_read(&buffer.read, buffer.page)
            .map_err(cache_err)?;
        let result = read(page.as_bytes());
        self.cache.unmap(buffer.page);
        Ok(result)
    }

    /// Revokes a buffer's capabilities: every later read/write through it
    /// is denied (the revocation is enforced by the cache, not by
    /// convention).
    pub fn revoke(&mut self, buffer: &SharedBuffer) {
        let mut issuer = self.issuer.borrow_mut();
        issuer.revoke(&buffer.read);
        issuer.revoke(&buffer.write);
    }
}

fn cache_err(err: CacheError) -> RuntimeError {
    match err {
        CacheError::Denied => RuntimeError::new(
            ErrorKind::CapabilityDenied,
            "shared buffer capability denied (revoked or foreign)",
        ),
        CacheError::Storage(err) => RuntimeError::new(
            ErrorKind::StorageFailure,
            format!("shared buffer storage failure: {err:?}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_round_trip_zero_copy_read() {
        let mut pool = SharedBufferPool::new(4).unwrap();
        assert_eq!(pool.pages(), 4);

        let buffer = pool.attach(2).unwrap();
        pool.write(&buffer, 0, b"shared-bytes").unwrap();
        let seen = pool.with_read(&buffer, |page| page[..12].to_vec()).unwrap();
        assert_eq!(&seen, b"shared-bytes");

        // Offset writes do not disturb earlier bytes.
        pool.write(&buffer, 16, b"x").unwrap();
        let seen = pool
            .with_read(&buffer, |page| (page[..17]).to_vec())
            .unwrap();
        assert_eq!(&seen[..12], b"shared-bytes");
        assert_eq!(seen[16], b'x');
    }

    #[test]
    fn wrong_page_and_revoked_capabilities_are_denied() {
        let mut pool = SharedBufferPool::new(2).unwrap();
        let buffer = pool.attach(1).unwrap();

        // A capability for page 1 does not open page 0: the tokens are
        // per-page, so minting for another page and using it directly
        // against page 1's tokens is denied by the cache.
        let forged = SharedBuffer { page: 0, ..buffer };
        let err = pool.with_read(&forged, |_| ()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::CapabilityDenied);

        // Revocation cuts off an existing holder.
        pool.revoke(&buffer);
        assert_eq!(
            pool.with_read(&buffer, |_| ()).unwrap_err().kind,
            ErrorKind::CapabilityDenied
        );
        assert_eq!(
            pool.write(&buffer, 0, b"x").unwrap_err().kind,
            ErrorKind::CapabilityDenied
        );
    }

    #[test]
    fn bounds_are_enforced() {
        let mut pool = SharedBufferPool::new(1).unwrap();
        assert!(pool.attach(1).is_err(), "page beyond the pool is refused");
        let buffer = pool.attach(0).unwrap();
        assert!(pool.write(&buffer, PAGE_SIZE - 1, b"ab").is_err());
    }
}
