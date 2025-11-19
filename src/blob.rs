use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum BlobStorageError {
    #[error("Sled backend error: {0}")]
    SledBackendError(#[from] sled::Error),
}

pub trait BlobStorage: Send + Sync {
    fn insert(&self, key: &[u8], value: &[u8]) -> Result<(), BlobStorageError>;
    fn get_by_key(&self, id: &[u8]) -> Result<Option<Vec<u8>>, BlobStorageError>;

    fn save(&self, data: &[u8]) -> Result<Uuid, BlobStorageError>;
    fn get(&self, id: Uuid) -> Result<Option<Vec<u8>>, BlobStorageError> {
        self.get_by_key(id.as_bytes())
    }

    fn remove(&self, key: &[u8]) -> Result<Option<Vec<u8>>, BlobStorageError>;
}

impl BlobStorage for sled::Tree {
    fn insert(&self, key: &[u8], value: &[u8]) -> Result<(), BlobStorageError> {
        self.insert(key, value)?;
        Ok(())
    }

    fn get_by_key(&self, key: &[u8]) -> Result<Option<Vec<u8>>, BlobStorageError> {
        Ok(self.get(key)?.map(|v| v.to_vec()))
    }

    fn remove(&self, key: &[u8]) -> Result<Option<Vec<u8>>, BlobStorageError> {
        self.remove(key)
            .map(|v| v.map(|v| v.to_vec()))
            .map_err(|e| BlobStorageError::SledBackendError(e))
    }

    fn save(&self, data: &[u8]) -> Result<Uuid, BlobStorageError> {
        let mut uuid = Uuid::new_v4();
        for _ in 0..10 {
            match self.compare_and_swap(uuid, None::<&[u8]>, Some(data))? {
                Ok(()) => break,
                Err(_) => {
                    uuid = Uuid::new_v4();
                }
            }
        }
        Ok(uuid)
    }
}
