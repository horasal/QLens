mod blob;
mod chat_handler;
mod schema;
mod session;
mod tools;

use std::sync::Arc;

pub use blob::*;
pub use chat_handler::*;
use redb::Database;
pub use schema::*;
use serde::{Deserialize, Serialize};
pub use session::*;
use strum::{Display, EnumIter, EnumString};
pub use tools::*;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, Display, EnumIter, Serialize, Deserialize,
)]
#[strum(serialize_all = "snake_case")]
pub enum StorageKind {
    #[strum(serialize = "sled")]
    Sled,
    #[strum(serialize = "redb")]
    Redb,
}

#[derive(Clone)]
pub struct Storages {
    history: Arc<dyn SessionStorage>,
    image: Arc<dyn BlobStorage>,
    asset: Arc<dyn BlobStorage>,
    memo: Arc<dyn BlobStorage>,
}

impl StorageKind {
    pub fn create_storages<T: AsRef<str>>(&self, path: T) -> Result<Storages, anyhow::Error> {
        match self {
            StorageKind::Redb => {
                tracing::info!("Use redb as storage backend.");
                let db = Arc::new(Database::create(path.as_ref())?);
                let history = Arc::new(RedbSessionStore::new(db.clone(), "history")?);
                let image = Arc::new(RedbBlobStorage::new(db.clone(), "image")?);
                let asset = Arc::new(RedbBlobStorage::new(db.clone(), "asset")?);
                let memo = Arc::new(RedbBlobStorage::new(db.clone(), "memo")?);
                Ok(Storages {
                    history,
                    image,
                    asset,
                    memo,
                })
            }
            StorageKind::Sled => {
                tracing::info!("Use sled as storage backend.");
                let db = sled::Config::new()
                    .temporary(false)
                    .path(path.as_ref())
                    .use_compression(true)
                    .open()?;
                let history = Arc::new(SledSessionStore::new_from_db(&db, "history")?);
                let image = Arc::new(SledBlobStorage::new_from_db(&db, "image")?);
                let asset = Arc::new(SledBlobStorage::new_from_db(&db, "asset")?);
                let memo = Arc::new(SledBlobStorage::new_from_db(&db, "memo")?);
                Ok(Storages {
                    history,
                    image,
                    asset,
                    memo,
                })
            }
        }
    }
}
