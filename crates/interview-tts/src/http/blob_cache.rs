//! 把合成缓存放进 BlobStore：键是 `tts-cache/{内容哈希}.wav`。

use async_trait::async_trait;
use bytes::Bytes;
use interview_platform::blob::SharedBlobStore;

use crate::cache::{AudioCache, CacheError, cache_blob_key};

/// 存在 BlobStore 里的合成缓存。服务重启后还在，同一句话、同一个音色不会合成第二次。
#[derive(Clone)]
pub struct BlobAudioCache {
    blobs: SharedBlobStore,
}

impl BlobAudioCache {
    pub fn new(blobs: SharedBlobStore) -> Self {
        Self { blobs }
    }
}

#[async_trait]
impl AudioCache for BlobAudioCache {
    async fn get(&self, content_hash: &str) -> Result<Option<Vec<u8>>, CacheError> {
        let key = cache_blob_key(content_hash);
        let exists = self
            .blobs
            .head(&key)
            .await
            .map_err(|error| CacheError(error.to_string()))?
            .is_some();
        if !exists {
            return Ok(None);
        }
        let bytes = self
            .blobs
            .get(&key)
            .await
            .map_err(|error| CacheError(error.to_string()))?;
        Ok(Some(bytes.to_vec()))
    }

    async fn put(&self, content_hash: &str, wav: &[u8]) -> Result<(), CacheError> {
        self.blobs
            .put(&cache_blob_key(content_hash), Bytes::copy_from_slice(wav))
            .await
            .map_err(|error| CacheError(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use interview_platform::blob::MemoryBlobStore;

    use super::*;

    #[tokio::test]
    async fn stores_under_the_agreed_key() {
        let blobs = Arc::new(MemoryBlobStore::new());
        let cache = BlobAudioCache::new(blobs.clone());
        assert_eq!(cache.get("abc").await.unwrap(), None);
        cache.put("abc", b"RIFF").await.unwrap();
        assert_eq!(
            cache.get("abc").await.unwrap().as_deref(),
            Some(&b"RIFF"[..])
        );
        assert_eq!(blobs.keys(), ["tts-cache/abc.wav"]);
    }
}
