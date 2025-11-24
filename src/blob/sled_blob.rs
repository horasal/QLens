use crate::{BlobStorage, BlobStorageError};
use sled::{Transactional, transaction::TransactionError};
use crate::AssetId;

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
    fn save(&self, data: &[u8]) -> Result<AssetId, BlobStorageError> {
        let uuid = AssetId::from_data(data);
        let key = uuid.as_bytes();
        let tx_result: Result<(), TransactionError<sled::Error>> = (&self.data_tree, &self.rc_tree)
            .transaction(|(d_tree, r_tree)| {
                if d_tree.get(key)?.is_some() {
                    let old_rc = r_tree
                        .get(key)?
                        .map(|v| u64::from_be_bytes(v[..8].try_into().unwrap()))
                        .unwrap_or(0);
                    r_tree.insert(key, &(old_rc + 1).to_be_bytes())?;
                    tracing::debug!("Duplicate Asset: {}", uuid);
                } else {
                    d_tree.insert(key, data)?;
                    r_tree.insert(key, &1u64.to_be_bytes())?;
                    tracing::debug!("Asset created: {}", uuid);
                }
                Ok(())
            });

        match tx_result {
            Ok(_) => return Ok(uuid),
            Err(sled::transaction::TransactionError::Abort(e)) => Err(e.into()),
            Err(e) => return Err(e.into()),
        }
    }

    fn retain(&self, uuid: AssetId) -> Result<(), BlobStorageError> {
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

    fn release(&self, uuid: AssetId) -> Result<bool, BlobStorageError> {
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

    fn peek_raw(&self, key: &[u8], n: usize) -> Result<Option<(Vec<u8>, usize)>, BlobStorageError> {
        Ok(self
            .data_tree
            .get(key)?
            .map(|iv| (iv.subslice(0, n.min(iv.len())).to_vec(), iv.len())))
    }
}
