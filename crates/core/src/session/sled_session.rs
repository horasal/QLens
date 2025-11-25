use crate::session::{SessionStorage, SessionStoreError};
use sled::{
    Transactional,
    transaction::{ConflictableTransactionError, TransactionError},
};
use uuid::Uuid;

pub struct SledSessionStore {
    meta_tree: sled::Tree,
    data_tree: sled::Tree,
}

impl SledSessionStore {
    pub fn new_from_db(db: &sled::Db, name: &str) -> Result<Self, SessionStoreError> {
        Ok(Self {
            meta_tree: db.open_tree(format!("{}_meta", name))?,
            data_tree: db.open_tree(format!("{}_data", name))?,
        })
    }
}

impl SessionStorage for SledSessionStore {
    fn append(&self, meta: &[u8], data: &[u8]) -> Result<Uuid, SessionStoreError> {
        for _ in 0..10 {
            let id = Uuid::now_v7();
            let key = id.as_bytes();
            let tx_result = (&self.meta_tree, &self.data_tree).transaction(|(t_meta, t_data)| {
                if t_meta.get(key)?.is_some() {
                    return Err(sled::transaction::ConflictableTransactionError::Abort(
                        SessionStoreError::UuidCollision,
                    ));
                }

                t_meta.insert(key, meta)?;
                t_data.insert(key, data)?;
                Ok(())
            });

            match tx_result {
                Ok(_) => return Ok(id),
                Err(sled::transaction::TransactionError::Abort(
                    SessionStoreError::UuidCollision,
                )) => {
                    continue;
                }
                Err(e) => return Err(SessionStoreError::SledTransactionError(e.to_string())),
            }
        }

        Err(SessionStoreError::UuidCollision)
    }

    fn update(&self, id: Uuid, meta: &[u8], data: &[u8]) -> Result<(), SessionStoreError> {
        let key = id.as_bytes();
        (&self.meta_tree, &self.data_tree)
            .transaction(|(t_meta, t_data)| {
                t_meta.insert(key, meta)?;
                t_data.insert(key, data)?;
                Ok(())
            })
            .map_err(|e: TransactionError<sled::Error>| {
                SessionStoreError::SledTransactionError(e.to_string())
            })?;
        Ok(())
    }

    fn get_meta(&self, id: Uuid) -> Result<Option<Vec<u8>>, SessionStoreError> {
        let val = self.meta_tree.get(id.as_bytes())?;
        Ok(val.map(|iv| iv.to_vec()))
    }

    fn get_data(&self, id: Uuid) -> Result<Option<Vec<u8>>, SessionStoreError> {
        let val = self.data_tree.get(id.as_bytes())?;
        Ok(val.map(|iv| iv.to_vec()))
    }

    fn list(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<(Uuid, Vec<u8>)>, SessionStoreError> {
        let iter = self.meta_tree.iter().rev();

        let skip = offset.unwrap_or(0);
        let take = limit.unwrap_or(usize::MAX);

        let mut result = Vec::new();

        for item in iter.skip(skip).take(take) {
            let (k, v) = item?;
            if k.len() == 16 {
                let uuid = Uuid::from_slice(&k).unwrap_or_default();
                result.push((uuid, v.to_vec()));
            }
        }

        Ok(result)
    }

    fn delete(&self, id: Uuid) -> Result<Option<Vec<u8>>, SessionStoreError> {
        let key = id.as_bytes();
        (&self.meta_tree, &self.data_tree)
            .transaction(|(t_meta, t_data)| {
                t_meta.remove(key)?;
                t_data
                    .remove(key)
                    .map(|v| v.map(|v| v.to_vec()))
                    .map_err(|e| e.into())
            })
            .map_err(|e: TransactionError<sled::Error>| {
                SessionStoreError::SledTransactionError(e.to_string())
            })
    }

    fn update_data_with(
        &self,
        id: Uuid,
        f: Box<
            dyn Fn(Option<Vec<u8>>, Option<Vec<u8>>) -> Result<(Vec<u8>, Vec<u8>), anyhow::Error>
                + Send,
        >,
    ) -> Result<(Vec<u8>, Vec<u8>), SessionStoreError> {
        let key = id.as_bytes();

        (&self.meta_tree, &self.data_tree)
            .transaction(|(t_meta, t_data)| {
                let old_meta = t_meta.get(key)?.map(|e| e.to_vec());
                let old_data = t_data.get(key)?.map(|e| e.to_vec());
                let (new_meta, new_data) = f(old_meta, old_data).map_err(|e| {
                    ConflictableTransactionError::Abort(SessionStoreError::CASInnerError(e))
                })?;
                t_meta.insert(key, new_meta.clone())?;
                t_data.insert(key, new_data.clone())?;
                Ok((new_meta, new_data))
            })
            .map_err(|e: TransactionError<SessionStoreError>| {
                SessionStoreError::SledTransactionError(e.to_string())
            })
    }
}
