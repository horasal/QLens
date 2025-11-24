use thiserror::Error;
use crate::AssetId;

#[derive(Debug, Error)]
pub enum BlobStorageError {
    #[error("Sled error: {0}")]
    SledError(#[from] sled::Error),
    #[error("Sled transaction error: {0}")]
    SledTransactionError(String),
    #[error("Data corruption: Invalid reference count bytes")]
    InvalidRefCountData,
    #[error("UUID generation failed after multiple retries")]
    UuidGenerationFailed,
}

impl<E> From<sled::transaction::TransactionError<E>> for BlobStorageError
where
    E: std::fmt::Display,
{
    fn from(e: sled::transaction::TransactionError<E>) -> Self {
        BlobStorageError::SledTransactionError(format!("{}", e))
    }
}

pub trait BlobStorage: Send + Sync {
    /// 保存新数据，返回新生成的 UUID。引用计数初始化为 1。
    fn save(&self, data: &[u8]) -> Result<AssetId, BlobStorageError>;

    /// 获取数据
    fn get(&self, uuid: AssetId) -> Result<Option<Vec<u8>>, BlobStorageError> {
        self.get_raw(uuid.as_bytes())
    }

    /// 增加引用计数 (复用uuid)
    fn retain(&self, uuid: AssetId) -> Result<(), BlobStorageError>;

    /// 减少引用计数
    /// 如果引用计数归零，则删除数据。
    /// 返回值: true 表示数据已被物理删除，false 表示仅减少了计数
    fn release(&self, uuid: AssetId) -> Result<bool, BlobStorageError>;

    fn peek(&self, uuid: AssetId, n: usize) -> Result<Option<(Vec<u8>, usize)>, BlobStorageError> {
        self.peek_raw(uuid.as_bytes(), n)
    }
    /// 获取前N个字节的数据
    /// 返回值：(总大小，前N个字节的数据)
    /// 如果N>总大小，则返回总大小的数据
    fn peek_raw(&self, key: &[u8], n: usize) -> Result<Option<(Vec<u8>, usize)>, BlobStorageError>;
    fn put_raw(&self, key: &[u8], value: &[u8]) -> Result<(), BlobStorageError>;
    fn get_raw(&self, key: &[u8]) -> Result<Option<Vec<u8>>, BlobStorageError>;
    fn delete_raw(&self, key: &[u8]) -> Result<(), BlobStorageError>;
}
