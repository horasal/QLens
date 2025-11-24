use crate::blob::{AssetId, BlobStorage, BlobStorageError};
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};

// Data 表: Key = 20字节 Hash, Value = 原始数据
type TableBlob<'a, 'b, 'c> = TableDefinition<'a, &'b [u8; 20], &'c [u8]>;
// RC 表: Key = 20字节 Hash, Value = u64 引用计数
type TableRc<'a, 'b> = TableDefinition<'a, &'b [u8; 20], u64>;

pub struct RedbBlobStorage<'a, 'b, 'c> {
    db: Database,
    tb_blob: TableBlob<'a, 'b, 'c>,
    tb_rc: TableRc<'a, 'b>,
}

impl RedbBlobStorage {
    pub fn new(path: &str, table_name: &str) -> Result<Self, anyhow::Error> {
        let db = Database::create(path)?;
        let tb_blob = TableDefinition::new(table_name);
        let tb_rc = TableDefinition::new(format!("{}.ref_count", table_name).as_str());
        // 初始化表（redb 需要显式建表）
        let write_txn = db.begin_write()?;
        {
            write_txn.open_table(tb_blob)?;
            write_txn.open_table(tb_rc)?;
        }
        write_txn.commit()?;
        Ok(Self { db, tb_rc, tb_blob })
    }
}

impl BlobStorage for RedbBlobStorage {
    fn save(&self, data: &[u8]) -> Result<AssetId, BlobStorageError> {
        let id = AssetId::from_data(data);
        let key = id.as_bytes(); // &[u8; 20]

        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;

        {
            let mut table_blobs = write_txn
                .open_table(self.tb_blob)
                .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
            let mut table_rc = write_txn
                .open_table(self.tb_rc)
                .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;

            // 检查是否存在
            let current_rc = table_rc
                .get(key)
                .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;

            if let Some(rc) = current_rc {
                // 已存在 (Deduplication) -> RC + 1
                let new_rc = rc.value() + 1;
                table_rc
                    .insert(key, new_rc)
                    .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
                tracing::debug!("Asset Deduplicated: {}", id);
            } else {
                // 新数据 -> 写入 Data + RC=1
                table_blobs
                    .insert(key, data)
                    .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
                table_rc
                    .insert(key, 1)
                    .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
                tracing::debug!("Asset Created: {}", id);
            }
        }

        // 提交事务
        write_txn
            .commit()
            .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;

        Ok(id)
    }

    fn peek(&self, id: AssetId, n: usize) -> Result<Option<(Vec<u8>, usize)>, BlobStorageError> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
        let table = read_txn
            .open_table(self.tb_blob)
            .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;

        let result = table
            .get(id.as_bytes())
            .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;

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
        let key = id.as_bytes();
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
        let deleted;

        {
            let mut table_blobs = write_txn
                .open_table(self.tb_blob)
                .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
            let mut table_rc = write_txn
                .open_table(self.tb_rc)
                .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;

            let current_rc = table_rc
                .get(key)
                .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?
                .map(|v| v.value())
                .unwrap_or(0);

            if current_rc <= 1 {
                // 归零，物理删除
                table_rc
                    .remove(key)
                    .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
                table_blobs
                    .remove(key)
                    .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
                deleted = true;
            } else {
                // 引用减一
                table_rc
                    .insert(key, current_rc - 1)
                    .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
                deleted = false;
            }
        }

        write_txn
            .commit()
            .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
        Ok(deleted)
    }

    fn retain(&self, id: AssetId) -> Result<(), BlobStorageError> {
        let key = id.as_bytes();
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
        {
            let mut table_rc = write_txn
                .open_table(self.tb_rc)
                .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
            let current = table_rc
                .get(key)
                .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?
                .map(|v| v.value())
                .unwrap_or(0);

            table_rc
                .insert(key, current + 1)
                .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
        }
        write_txn
            .commit()
            .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
        Ok(())
    }

    fn get_raw(&self, key: &[u8]) -> Result<Option<Vec<u8>>, BlobStorageError> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;
        let table = read_txn
            .open_table(self.tb_blob)
            .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;

        let result = table
            .get(key)
            .map_err(|e| BlobStorageError::SledTransactionError(e.to_string()))?;

        Ok(result.map(|v| v.value().to_vec()))
    }
}
