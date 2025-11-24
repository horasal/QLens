use sled::{Transactional, transaction::TransactionError};
use thiserror::Error;
use uuid::Uuid;

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
    fn save(&self, data: &[u8]) -> Result<Uuid, BlobStorageError>;

    /// 获取数据
    fn get(&self, uuid: Uuid) -> Result<Option<Vec<u8>>, BlobStorageError>;

    /// 增加引用计数 (复用uuid)
    fn retain(&self, uuid: Uuid) -> Result<(), BlobStorageError>;

    /// 减少引用计数
    /// 如果引用计数归零，则删除数据。
    /// 返回值: true 表示数据已被物理删除，false 表示仅减少了计数
    fn release(&self, uuid: Uuid) -> Result<bool, BlobStorageError>;

    fn put_raw(&self, key: &[u8], value: &[u8]) -> Result<(), BlobStorageError>;
    fn get_raw(&self, key: &[u8]) -> Result<Option<Vec<u8>>, BlobStorageError>;
    fn delete_raw(&self, key: &[u8]) -> Result<(), BlobStorageError>;
}

#[derive(Clone)]
pub struct SledBlobStorage {
    data_tree: sled::Tree,
    // Key 是 UUID，Value 是 u64 (8 bytes)
    rc_tree: sled::Tree,
}

impl SledBlobStorage {
    pub fn new_from_db(db: &sled::Db, name: &str) -> Result<Self, sled::Error> {
        Ok(Self {
            data_tree: db.open_tree(name)?,
            rc_tree: db.open_tree(format!("{}_rc", name))?,
        })
    }

    #[allow(dead_code)]
    pub fn new_from_tree(data_tree: sled::Tree, rc_tree: sled::Tree) -> Self {
        Self { data_tree, rc_tree }
    }
}

impl BlobStorage for SledBlobStorage {
    fn get(&self, uuid: Uuid) -> Result<Option<Vec<u8>>, BlobStorageError> {
        let key = uuid.as_bytes();
        Ok(self.data_tree.get(key)?.map(|iv| iv.to_vec()))
    }

    fn save(&self, data: &[u8]) -> Result<Uuid, BlobStorageError> {
        // 尝试生成 UUID 的循环
        for _ in 0..10 {
            let uuid = Uuid::new_v4();
            let key = uuid.as_bytes();

            // 开启事务：同时写入 Data 和 RC
            let tx_result = (&self.data_tree, &self.rc_tree).transaction(|(d_tree, r_tree)| {
                if d_tree.get(key)?.is_some() {
                    // UUID 冲突，回滚并重试
                    return Err(sled::transaction::ConflictableTransactionError::Abort(
                        "UUID Collision",
                    ));
                }

                // 写入数据
                d_tree.insert(key, data)?;
                // 写入引用计数，初始为 1 (u64 big endian)
                r_tree.insert(key, &1u64.to_be_bytes())?;

                Ok(())
            });

            match tx_result {
                Ok(_) => return Ok(uuid),
                Err(sled::transaction::TransactionError::Abort(_)) => continue, // 重试
                Err(e) => return Err(e.into()),
            }
        }
        Err(BlobStorageError::UuidGenerationFailed)
    }

    fn retain(&self, uuid: Uuid) -> Result<(), BlobStorageError> {
        let key = uuid.as_bytes();

        self.rc_tree.update_and_fetch(key, |old_val| {
            let current = old_val
                .map(|v| {
                    let mut bytes = [0u8; 8];
                    bytes.copy_from_slice(v);
                    u64::from_be_bytes(bytes)
                })
                .unwrap_or(0);

            Some(u64::to_be_bytes(current + 1).to_vec())
        })?;

        Ok(())
    }

    fn release(&self, uuid: Uuid) -> Result<bool, BlobStorageError> {
        let key = uuid.as_bytes();

        let tx_result: Result<bool, TransactionError<sled::Error>> =
            (&self.data_tree, &self.rc_tree).transaction(|(d_tree, r_tree)| {
                let rc_val = r_tree.get(key)?;

                if let Some(val) = rc_val {
                    let mut bytes = [0u8; 8];
                    bytes.copy_from_slice(&val);
                    let count = u64::from_be_bytes(bytes);

                    if count <= 1 {
                        // 引用归零：删除 RC 和 Data
                        r_tree.remove(key)?;
                        d_tree.remove(key)?;
                        Ok(true) // 返回 true 表示已物理删除
                    } else {
                        // 引用减一
                        r_tree.insert(key, &u64::to_be_bytes(count - 1))?;
                        Ok(false) // 返回 false 表示数据还在
                    }
                } else {
                    // 数据不存在，视作不需要操作
                    Ok(false)
                }
            });

        Ok(tx_result?)
    }

    fn put_raw(&self, key: &[u8], value: &[u8]) -> Result<(), BlobStorageError> {
        self.data_tree.insert(key, value)?;
        Ok(())
    }

    fn get_raw(&self, key: &[u8]) -> Result<Option<Vec<u8>>, BlobStorageError> {
        Ok(self.data_tree.get(key)?.map(|v| v.to_vec()))
    }

    fn delete_raw(&self, key: &[u8]) -> Result<(), BlobStorageError> {
        self.data_tree.remove(key)?;
        Ok(())
    }
}
