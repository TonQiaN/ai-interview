//! 文件存储。简历、音频、录像都走这里，库里只存键。
//!
//! 键的拼法在 `interview_core::blob_key`。现在有两个实现：
//! [`LocalBlobStore`]（本地磁盘，正式用）和 [`MemoryBlobStore`]（内存，测试用）。
//! 以后要换成 S3 兼容的对象存储，再实现一遍 [`BlobStore`] 就行，调用方不用改。

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use bytes::Bytes;
use interview_core::blob_key;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

/// 文件存储出错。
#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("文件不存在：{0}")]
    NotFound(String),
    #[error(transparent)]
    InvalidKey(#[from] blob_key::InvalidKey),
    #[error("读取范围超出文件大小（文件 {size} 字节）")]
    RangeNotSatisfiable { size: u64 },
    #[error("文件存储读写失败：{0}")]
    Io(#[from] std::io::Error),
}

/// 一个文件的基本信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobMeta {
    pub key: String,
    pub size: u64,
}

/// 文件存储。
#[async_trait]
pub trait BlobStore: Send + Sync + 'static {
    /// 写入（覆盖已有的）。
    async fn put(&self, key: &str, data: Bytes) -> Result<(), BlobError>;

    /// 读出整个文件。大文件（录像）用 [`BlobStore::get_range`] 分段读。
    async fn get(&self, key: &str) -> Result<Bytes, BlobError>;

    /// 读出一段：`range` 是字节偏移，左闭右开。
    async fn get_range(&self, key: &str, range: Range<u64>) -> Result<Bytes, BlobError>;

    /// 文件大小等信息；不存在返回 `None`。
    async fn head(&self, key: &str) -> Result<Option<BlobMeta>, BlobError>;

    /// 删除。文件本来就不存在也算成功。
    async fn delete(&self, key: &str) -> Result<(), BlobError>;

    /// 列出某个前缀下的全部文件，按键排序。`prefix` 是完整的一段或几段，比如
    /// `interviews/{id}/recordings/camera/0/chunks`。
    async fn list(&self, prefix: &str) -> Result<Vec<BlobMeta>, BlobError>;

    /// 把几个文件按顺序拼成一个新文件（合并录制分片用）。
    async fn concat(&self, sources: &[String], target: &str) -> Result<BlobMeta, BlobError>;

    /// 这个文件在本机磁盘上的路径。要把文件交给 ffmpeg 这类外部程序时用；
    /// 存储不在本机磁盘上时返回 `None`，调用方要自己先下载到临时文件。
    fn local_path(&self, key: &str) -> Option<PathBuf>;
}

/// 传给各模块用的存储句柄。
pub type SharedBlobStore = Arc<dyn BlobStore>;

// ───────────────────────── 本地磁盘 ─────────────────────────

/// 把文件存在本机的一个目录下，键直接对应相对路径。
#[derive(Debug, Clone)]
pub struct LocalBlobStore {
    root: PathBuf,
}

impl LocalBlobStore {
    /// `root` 不存在会自动创建。
    pub async fn open(root: impl Into<PathBuf>) -> Result<Self, BlobError> {
        let root = root.into();
        tokio::fs::create_dir_all(&root).await?;
        let root = tokio::fs::canonicalize(&root).await?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path_of(&self, key: &str) -> Result<PathBuf, BlobError> {
        blob_key::validate(key)?;
        Ok(self.root.join(key))
    }
}

fn not_found_or_io(key: &str, err: std::io::Error) -> BlobError {
    if err.kind() == std::io::ErrorKind::NotFound {
        BlobError::NotFound(key.to_owned())
    } else {
        BlobError::Io(err)
    }
}

#[async_trait]
impl BlobStore for LocalBlobStore {
    async fn put(&self, key: &str, data: Bytes) -> Result<(), BlobError> {
        let path = self.path_of(key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // 先写临时文件再改名：读的人不会看到写了一半的文件
        let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7().simple()));
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(&data).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        let path = self.path_of(key)?;
        let data = tokio::fs::read(&path)
            .await
            .map_err(|e| not_found_or_io(key, e))?;
        Ok(Bytes::from(data))
    }

    async fn get_range(&self, key: &str, range: Range<u64>) -> Result<Bytes, BlobError> {
        let path = self.path_of(key)?;
        let mut file = tokio::fs::File::open(&path)
            .await
            .map_err(|e| not_found_or_io(key, e))?;
        let size = file.metadata().await?.len();
        if range.start > range.end || range.end > size {
            return Err(BlobError::RangeNotSatisfiable { size });
        }
        file.seek(std::io::SeekFrom::Start(range.start)).await?;
        let mut buf = vec![0u8; (range.end - range.start) as usize];
        file.read_exact(&mut buf).await?;
        Ok(Bytes::from(buf))
    }

    async fn head(&self, key: &str) -> Result<Option<BlobMeta>, BlobError> {
        let path = self.path_of(key)?;
        match tokio::fs::metadata(&path).await {
            Ok(meta) if meta.is_file() => Ok(Some(BlobMeta {
                key: key.to_owned(),
                size: meta.len(),
            })),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        let path = self.path_of(key)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    async fn list(&self, prefix: &str) -> Result<Vec<BlobMeta>, BlobError> {
        let dir = self.path_of(prefix)?;
        let mut found = Vec::new();
        let mut pending = vec![(dir, prefix.to_owned())];
        while let Some((dir, key_prefix)) = pending.pop() {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name().to_string_lossy().into_owned();
                let key = format!("{key_prefix}/{name}");
                // 写了一半的临时文件、不合约定的文件名都跳过
                if blob_key::validate(&key).is_err() || name.contains(".tmp-") {
                    continue;
                }
                let meta = entry.metadata().await?;
                if meta.is_dir() {
                    pending.push((entry.path(), key));
                } else if meta.is_file() {
                    found.push(BlobMeta {
                        key,
                        size: meta.len(),
                    });
                }
            }
        }
        found.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(found)
    }

    async fn concat(&self, sources: &[String], target: &str) -> Result<BlobMeta, BlobError> {
        let target_path = self.path_of(target)?;
        if let Some(parent) = target_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = target_path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7().simple()));
        let mut out = tokio::fs::File::create(&tmp).await?;
        let mut size = 0u64;
        for source in sources {
            let path = self.path_of(source)?;
            let mut input = match tokio::fs::File::open(&path).await {
                Ok(file) => file,
                Err(e) => {
                    drop(out);
                    let _ = tokio::fs::remove_file(&tmp).await;
                    return Err(not_found_or_io(source, e));
                }
            };
            size += tokio::io::copy(&mut input, &mut out).await?;
        }
        out.sync_all().await?;
        drop(out);
        tokio::fs::rename(&tmp, &target_path).await?;
        Ok(BlobMeta {
            key: target.to_owned(),
            size,
        })
    }

    fn local_path(&self, key: &str) -> Option<PathBuf> {
        self.path_of(key).ok()
    }
}

// ───────────────────────── 内存（测试用） ─────────────────────────

/// 把文件放在内存里。只给测试用。
#[derive(Debug, Default, Clone)]
pub struct MemoryBlobStore {
    files: Arc<RwLock<BTreeMap<String, Bytes>>>,
}

impl MemoryBlobStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 现在存了哪些键。
    pub fn keys(&self) -> Vec<String> {
        self.read().keys().cloned().collect()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<String, Bytes>> {
        self.files
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, BTreeMap<String, Bytes>> {
        self.files
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl BlobStore for MemoryBlobStore {
    async fn put(&self, key: &str, data: Bytes) -> Result<(), BlobError> {
        blob_key::validate(key)?;
        self.write().insert(key.to_owned(), data);
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        blob_key::validate(key)?;
        self.read()
            .get(key)
            .cloned()
            .ok_or_else(|| BlobError::NotFound(key.to_owned()))
    }

    async fn get_range(&self, key: &str, range: Range<u64>) -> Result<Bytes, BlobError> {
        let data = self.get(key).await?;
        let size = data.len() as u64;
        if range.start > range.end || range.end > size {
            return Err(BlobError::RangeNotSatisfiable { size });
        }
        Ok(data.slice(range.start as usize..range.end as usize))
    }

    async fn head(&self, key: &str) -> Result<Option<BlobMeta>, BlobError> {
        blob_key::validate(key)?;
        Ok(self.read().get(key).map(|d| BlobMeta {
            key: key.to_owned(),
            size: d.len() as u64,
        }))
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        blob_key::validate(key)?;
        self.write().remove(key);
        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<BlobMeta>, BlobError> {
        blob_key::validate(prefix)?;
        let wanted = format!("{prefix}/");
        Ok(self
            .read()
            .iter()
            .filter(|(key, _)| key.starts_with(&wanted))
            .map(|(key, data)| BlobMeta {
                key: key.clone(),
                size: data.len() as u64,
            })
            .collect())
    }

    async fn concat(&self, sources: &[String], target: &str) -> Result<BlobMeta, BlobError> {
        blob_key::validate(target)?;
        let mut joined = Vec::new();
        for source in sources {
            joined.extend_from_slice(&self.get(source).await?);
        }
        let size = joined.len() as u64;
        self.write().insert(target.to_owned(), Bytes::from(joined));
        Ok(BlobMeta {
            key: target.to_owned(),
            size,
        })
    }

    fn local_path(&self, _key: &str) -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 两个实现要表现一致，同一套检查各跑一遍。
    async fn exercise(store: &dyn BlobStore) {
        let key = "interviews/abc/utterances/q1.wav";
        assert_eq!(store.head(key).await.unwrap(), None);
        assert!(matches!(store.get(key).await, Err(BlobError::NotFound(_))));

        store
            .put(key, Bytes::from_static(b"0123456789"))
            .await
            .unwrap();
        assert_eq!(
            store.get(key).await.unwrap(),
            Bytes::from_static(b"0123456789")
        );
        assert_eq!(store.head(key).await.unwrap().unwrap().size, 10);
        assert_eq!(
            store.get_range(key, 2..5).await.unwrap(),
            Bytes::from_static(b"234")
        );
        assert_eq!(store.get_range(key, 10..10).await.unwrap().len(), 0);
        assert!(matches!(
            store.get_range(key, 5..11).await,
            Err(BlobError::RangeNotSatisfiable { size: 10 })
        ));

        // 覆盖
        store.put(key, Bytes::from_static(b"new")).await.unwrap();
        assert_eq!(store.get(key).await.unwrap(), Bytes::from_static(b"new"));

        // 分片：列出来是按键排序的，拼起来顺序由调用方决定
        for (seq, body) in [(0, "aa"), (1, "bb"), (2, "cc")] {
            let chunk = format!("interviews/abc/recordings/camera/0/chunks/{seq}");
            store.put(&chunk, Bytes::from(body)).await.unwrap();
        }
        let chunks = store
            .list("interviews/abc/recordings/camera/0/chunks")
            .await
            .unwrap();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].key, "interviews/abc/recordings/camera/0/chunks/0");
        let sources: Vec<String> = chunks.into_iter().map(|c| c.key).collect();
        let target = "interviews/abc/recordings/camera/0/recording.webm";
        let merged = store.concat(&sources, target).await.unwrap();
        assert_eq!(merged.size, 6);
        assert_eq!(
            store.get(target).await.unwrap(),
            Bytes::from_static(b"aabbcc")
        );

        // 整个面试下面的文件都能列出来
        let all = store.list("interviews/abc").await.unwrap();
        assert_eq!(all.len(), 5);
        assert!(store.list("interviews/none").await.unwrap().is_empty());

        // 拼接时缺一个源文件要报错，而且不留下目标文件
        let missing = vec!["interviews/abc/nope".to_owned()];
        assert!(matches!(
            store.concat(&missing, "interviews/abc/out.bin").await,
            Err(BlobError::NotFound(_))
        ));
        assert_eq!(store.head("interviews/abc/out.bin").await.unwrap(), None);

        store.delete(key).await.unwrap();
        store.delete(key).await.unwrap();
        assert_eq!(store.head(key).await.unwrap(), None);

        // 不合法的键一律拒绝
        assert!(matches!(
            store.get("../etc/passwd").await,
            Err(BlobError::InvalidKey(_))
        ));
        assert!(matches!(
            store.put("a/../../b", Bytes::new()).await,
            Err(BlobError::InvalidKey(_))
        ));
    }

    #[tokio::test]
    async fn memory_store_behaves() {
        exercise(&MemoryBlobStore::new()).await;
    }

    #[tokio::test]
    async fn local_store_behaves() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalBlobStore::open(dir.path().join("blobs"))
            .await
            .unwrap();
        exercise(&store).await;
    }

    #[tokio::test]
    async fn local_store_exposes_paths_for_external_tools() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalBlobStore::open(dir.path()).await.unwrap();
        store
            .put("voices/v1/reference.wav", Bytes::from_static(b"RIFF"))
            .await
            .unwrap();
        let path = store.local_path("voices/v1/reference.wav").unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"RIFF");
        assert!(store.local_path("../outside").is_none());
    }
}
