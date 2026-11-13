use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;

/// 合成结果的缓存。键是 [`crate::SynthesisRequest::content_hash`]，值是 WAV 字节。
///
/// 同一道题、同一个音色只合成一次：换一场面试、重新生成题目列表时，没变的题目直接用缓存。
/// 缓存读写失败不会让合成失败，只是这一次不走缓存。
#[async_trait]
pub trait AudioCache: Send + Sync {
    async fn get(&self, content_hash: &str) -> Result<Option<Vec<u8>>, CacheError>;
    async fn put(&self, content_hash: &str, wav: &[u8]) -> Result<(), CacheError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("合成缓存读写失败：{0}")]
pub struct CacheError(pub String);

/// 约定的缓存对象键：`tts-cache/{内容哈希}.wav`。接 BlobStore 的缓存也用这个键。
pub fn cache_blob_key(content_hash: &str) -> String {
    format!("tts-cache/{content_hash}.wav")
}

/// 放在内存里的缓存，进程退出就没了。测试和命令行试用时用。
#[derive(Debug, Default)]
pub struct MemoryCache {
    entries: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemoryCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl AudioCache for MemoryCache {
    async fn get(&self, content_hash: &str) -> Result<Option<Vec<u8>>, CacheError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| CacheError("内存缓存的锁坏了".to_owned()))?;
        Ok(entries.get(content_hash).cloned())
    }

    async fn put(&self, content_hash: &str, wav: &[u8]) -> Result<(), CacheError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CacheError("内存缓存的锁坏了".to_owned()))?;
        entries.insert(content_hash.to_owned(), wav.to_vec());
        Ok(())
    }
}

/// 放在本地目录里的缓存，一个哈希一个 WAV 文件。
///
/// 写的时候先写到临时文件再改名，进程中途被杀也不会留下半个文件。
#[derive(Debug, Clone)]
pub struct DirCache {
    dir: PathBuf,
}

impl DirCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_for(&self, content_hash: &str) -> Result<PathBuf, CacheError> {
        // 哈希会变成文件名，只接受十六进制，免得拼出目录外的路径
        let is_hex =
            !content_hash.is_empty() && content_hash.bytes().all(|b| b.is_ascii_hexdigit());
        if !is_hex {
            return Err(CacheError(format!("内容哈希不是十六进制：{content_hash}")));
        }
        Ok(self.dir.join(format!("{content_hash}.wav")))
    }
}

#[async_trait]
impl AudioCache for DirCache {
    async fn get(&self, content_hash: &str) -> Result<Option<Vec<u8>>, CacheError> {
        let path = self.path_for(content_hash)?;
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(CacheError(format!("读 {} 失败：{e}", path.display()))),
        }
    }

    async fn put(&self, content_hash: &str, wav: &[u8]) -> Result<(), CacheError> {
        let path = self.path_for(content_hash)?;
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|e| CacheError(format!("建目录 {} 失败：{e}", self.dir.display())))?;
        let tmp = self
            .dir
            .join(format!("{content_hash}.{}.tmp", std::process::id()));
        tokio::fs::write(&tmp, wav)
            .await
            .map_err(|e| CacheError(format!("写 {} 失败：{e}", tmp.display())))?;
        tokio::fs::rename(&tmp, &path)
            .await
            .map_err(|e| CacheError(format!("改名到 {} 失败：{e}", path.display())))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "interview-tts-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[tokio::test]
    async fn memory_cache_returns_what_was_put() {
        let cache = MemoryCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.get("abc").await, Ok(None));
        cache.put("abc", b"wav bytes").await.unwrap();
        assert_eq!(cache.get("abc").await, Ok(Some(b"wav bytes".to_vec())));
        assert_eq!(cache.len(), 1);
    }

    #[tokio::test]
    async fn dir_cache_survives_a_new_instance_and_leaves_no_temp_files() {
        let dir = temp_dir("dir-cache");
        let cache = DirCache::new(&dir);
        assert_eq!(cache.get("0a1b").await, Ok(None));
        cache.put("0a1b", b"first").await.unwrap();
        cache.put("0a1b", b"second").await.unwrap();

        let reopened = DirCache::new(&dir);
        assert_eq!(reopened.get("0a1b").await, Ok(Some(b"second".to_vec())));
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["0a1b.wav"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn dir_cache_rejects_keys_that_are_not_hashes() {
        let cache = DirCache::new(temp_dir("bad-key"));
        for key in ["../../etc/passwd", "", "abc/def", "xyz"] {
            assert!(cache.get(key).await.is_err(), "{key}");
            assert!(cache.put(key, b"x").await.is_err(), "{key}");
        }
    }

    #[test]
    fn blob_key_follows_the_architecture_contract() {
        assert_eq!(cache_blob_key("0a1b"), "tts-cache/0a1b.wav");
    }
}
