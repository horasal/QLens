use uuid::Uuid;

#[derive(thiserror::Error, Debug)]
pub enum SessionStoreError {
    #[error("UUID collision")]
    UuidCollision,
    #[error("Redb error: {0}")]
    RedbError(#[from] redb::Error),
    #[error("Redb Commit error: {0}")]
    RedbCommitStorageError(#[from] redb::CommitError),
    #[error("Redb Transaction error: {0}")]
    RedbTransactionStorageError(#[from] redb::TransactionError),
    #[error("Redb Table error: {0}")]
    RedbTableError(#[from] redb::TableError),
    #[error("Redb Storage error: {0}")]
    RedbStorageError(#[from] redb::StorageError),
    #[error("Sled error: {0}")]
    SledError(#[from] sled::Error),
    #[error("Sled transaction error: {0}")]
    SledTransactionError(String),
    #[error("Cas Inner function error: {0}")]
    CASInnerError(#[from] anyhow::Error),
}

pub trait SessionStorage: Send + Sync {
    fn append(&self, meta: &[u8], data: &[u8]) -> Result<Uuid, SessionStoreError>;
    fn update(&self, id: Uuid, meta: &[u8], data: &[u8]) -> Result<(), SessionStoreError>;
    fn get_meta(&self, id: Uuid) -> Result<Option<Vec<u8>>, SessionStoreError>;
    fn get_data(&self, id: Uuid) -> Result<Option<Vec<u8>>, SessionStoreError>;
    fn list(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<(Uuid, Vec<u8>)>, SessionStoreError>;
    fn delete(&self, id: Uuid) -> Result<Option<Vec<u8>>, SessionStoreError>;
    fn update_data_with(
        &self,
        id: Uuid,
        f: Box<dyn Fn(Option<Vec<u8>>, Option<Vec<u8>>) -> Result<(Vec<u8>, Vec<u8>), anyhow::Error> + Send>
    ) -> Result<(Vec<u8>, Vec<u8>), SessionStoreError>;
}
