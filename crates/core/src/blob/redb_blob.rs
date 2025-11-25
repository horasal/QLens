use std::sync::Arc;

use crate::blob::{AssetId, BlobStorage, BlobStorageError};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

// Data 表: Key = 20字节 Hash, Value = 原始数据
type TableBlob<'a, 'b, 'c> = TableDefinition<'a, &'b [u8], &'c [u8]>;
// RC 表: Key = 20字节 Hash, Value = u64 引用计数
type TableRc<'a, 'b> = TableDefinition<'a, &'b [u8], u64>;

pub struct RedbBlobStorage {
    db: Arc<Database>,
    blob_name: String,
    rc_name: String,
}

impl RedbBlobStorage {
    pub fn new(db: Arc<Database>, table_name: &str) -> Result<Self, anyhow::Error> {
        let rc_name = format!("{}.reference_count", table_name);
        // 初始化表（redb 需要显式建表）
        let write_txn = db.begin_write()?;
        {
            let tb_blob = TableBlob::new(table_name);
            let tb_rc = TableRc::new(rc_name.as_str());
            write_txn.open_table(tb_blob)?;
            write_txn.open_table(tb_rc)?;
        }
        write_txn.commit()?;
        Ok(Self {
            db,
            blob_name: table_name.to_string(),
            rc_name,
        })
    }
}

impl BlobStorage for RedbBlobStorage {
    fn save(&self, data: &[u8]) -> Result<AssetId, BlobStorageError> {
        let id = AssetId::from_data(data);
        let key: &[u8] = id.as_bytes().as_slice();

        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;

        {
            let tb_blob = TableBlob::new(&self.blob_name);
            let tb_rc = TableRc::new(&self.rc_name);
            let mut table_blobs = write_txn
                .open_table(tb_blob)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
            let mut table_rc = write_txn
                .open_table(tb_rc)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;

            // 检查是否存在
            let rc = {
                let current_rc = table_rc
                    .get(key)
                    .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;

                if let Some(rc) = current_rc {
                    // 已存在 (Deduplication) -> RC + 1
                    tracing::debug!("Asset Deduplicated: {}", id);
                    rc.value() + 1
                } else {
                    // 新数据 -> 写入 Data + RC=1
                    table_blobs
                        .insert(key, data)
                        .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
                    tracing::debug!("Asset Created: {}", id);
                    1
                }
            };
            table_rc
                .insert(key, rc)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
        }

        // 提交事务
        write_txn
            .commit()
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;

        Ok(id)
    }

    fn peek_raw(&self, id: &[u8], n: usize) -> Result<Option<(Vec<u8>, usize)>, BlobStorageError> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
        let tb_blob = TableBlob::new(&self.blob_name);
        let table = read_txn
            .open_table(tb_blob)
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;

        let result = table
            .get(id)
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;

        match result {
            Some(access) => {
                let val = access.value();
                let total = val.len();
                let peek_len = n.min(total);
                let preview = val[..peek_len].to_vec();
                Ok(Some((preview, total)))
            }
            None => Ok(None),
        }
    }

    fn release(&self, id: AssetId) -> Result<bool, BlobStorageError> {
        let key = id.as_bytes().as_slice();
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
        let deleted;

        {
            let tb_blob = TableBlob::new(&self.blob_name);
            let tb_rc = TableRc::new(&self.rc_name);
            let mut table_blobs = write_txn
                .open_table(tb_blob)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
            let mut table_rc = write_txn
                .open_table(tb_rc)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;

            let current_rc = table_rc
                .get(key)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?
                .map(|v| v.value())
                .unwrap_or(0);

            if current_rc <= 1 {
                // 归零，物理删除
                table_rc
                    .remove(key)
                    .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
                table_blobs
                    .remove(key)
                    .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
                deleted = true;
            } else {
                // 引用减一
                table_rc
                    .insert(key, current_rc - 1)
                    .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
                deleted = false;
            }
        }

        write_txn
            .commit()
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
        Ok(deleted)
    }

    fn retain(&self, id: AssetId) -> Result<(), BlobStorageError> {
        let key = id.as_bytes().as_slice();
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
        {
            let tb_rc = TableRc::new(&self.rc_name);
            let mut table_rc = write_txn
                .open_table(tb_rc)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
            let current = table_rc
                .get(key)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?
                .map(|v| v.value())
                .unwrap_or(0);

            table_rc
                .insert(key, current + 1)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
        }
        write_txn
            .commit()
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
        Ok(())
    }

    fn get_raw(&self, key: &[u8]) -> Result<Option<Vec<u8>>, BlobStorageError> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
        let tb_blob = TableBlob::new(&self.blob_name);
        let table = read_txn
            .open_table(tb_blob)
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;

        let result = table
            .get(key)
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;

        Ok(result.map(|v| v.value().to_vec()))
    }

    fn delete_raw(&self, key: &[u8]) -> Result<(), BlobStorageError> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
        {
            let tb_blob = TableBlob::new(&self.blob_name);
            let tb_rc = TableRc::new(&self.rc_name);
            let mut table_blobs = write_txn
                .open_table(tb_blob)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
            let mut table_rc = write_txn
                .open_table(tb_rc)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
            table_blobs
                .remove(key)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
            let _ = table_rc
                .remove(key)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
        }
        write_txn
            .commit()
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
        Ok(())
    }

    fn put_raw(&self, key: &[u8], value: &[u8]) -> Result<(), BlobStorageError> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;

        {
            let tb_blob = TableBlob::new(&self.blob_name);
            let mut table_blobs = write_txn
                .open_table(tb_blob)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
            table_blobs
                .insert(key, value)
                .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
        }
        write_txn
            .commit()
            .map_err(|e| BlobStorageError::StorageTransactionError(e.to_string()))?;
        Ok(())
    }
}
