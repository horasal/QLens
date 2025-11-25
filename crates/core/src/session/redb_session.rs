use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use std::sync::Arc;
use uuid::Uuid;

use crate::SessionStoreError;

type TableMeta<'a, 'b, 'c> = TableDefinition<'a, &'b [u8; 16], &'c [u8]>;
type TableData<'a, 'b, 'c> = TableDefinition<'a, &'b [u8; 16], &'c [u8]>;

pub struct RedbSessionStore {
    data_table_name: String,
    meta_table_name: String,
    db: Arc<Database>,
}

impl RedbSessionStore {
    pub fn new(db: Arc<Database>, table_name: &str) -> Result<Self, SessionStoreError> {
        let write_txn = db.begin_write()?;
        let data_table_name = format!("{}_data", table_name);
        let meta_table_name = format!("{}_meta", table_name);
        {
            let tb_meta = TableMeta::new(&meta_table_name);
            let tb_data = TableData::new(&data_table_name);
            write_txn.open_table(tb_meta)?;
            write_txn.open_table(tb_data)?;
        }
        write_txn.commit()?;
        Ok(Self {
            data_table_name,
            meta_table_name,
            db,
        })
    }
}

impl super::SessionStorage for RedbSessionStore {
    fn append(&self, meta: &[u8], data: &[u8]) -> Result<Uuid, SessionStoreError> {
        for _ in 0..10 {
            let id = Uuid::now_v7();
            let key = id.as_bytes();
            let write_txn = self.db.begin_write()?;
            {
                let tb_meta = TableMeta::new(&self.meta_table_name);
                let tb_data = TableData::new(&self.data_table_name);
                let mut tb_meta = write_txn.open_table(tb_meta)?;
                let mut tb_data = write_txn.open_table(tb_data)?;

                if tb_meta.get(key)?.is_some() {
                    continue;
                }

                tb_meta.insert(key, meta)?;
                tb_data.insert(key, data)?;
            }
            write_txn.commit()?;
            return Ok(id);
        }
        Err(SessionStoreError::UuidCollision)
    }

    fn update(&self, id: Uuid, meta: &[u8], data: &[u8]) -> Result<(), SessionStoreError> {
        let key = id.as_bytes();
        let write_txn = self.db.begin_write()?;
        {
            let tb_meta = TableMeta::new(&self.meta_table_name);
            let tb_data = TableData::new(&self.data_table_name);
            let mut tb_meta = write_txn.open_table(tb_meta)?;
            let mut tb_data = write_txn.open_table(tb_data)?;

            tb_meta.insert(key, meta)?;
            tb_data.insert(key, data)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    fn get_meta(&self, id: Uuid) -> Result<Option<Vec<u8>>, SessionStoreError> {
        let read_txn = self.db.begin_read()?;
        let tb_meta = TableMeta::new(&self.meta_table_name);
        let tb_meta = read_txn.open_table(tb_meta)?;
        let result = tb_meta.get(id.as_bytes())?;
        Ok(result.map(|v| v.value().to_vec()))
    }

    fn get_data(&self, id: Uuid) -> Result<Option<Vec<u8>>, SessionStoreError> {
        let read_txn = self.db.begin_read()?;
        let tb_data = TableData::new(&self.data_table_name);
        let tb_data = read_txn.open_table(tb_data)?;
        let result = tb_data.get(id.as_bytes())?;
        Ok(result.map(|v| v.value().to_vec()))
    }

    fn list(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<(Uuid, Vec<u8>)>, SessionStoreError> {
        let read_txn = self.db.begin_read()?;
        let tb_meta = TableMeta::new(&self.meta_table_name);
        let tb_meta = read_txn.open_table(tb_meta)?;

        let iter = tb_meta.iter()?.rev();

        let skip = offset.unwrap_or(0);
        let take = limit.unwrap_or(usize::MAX);

        let mut result = Vec::new();

        for item in iter.skip(skip).take(take) {
            let (k_access, v_access) = item?;
            let id = Uuid::from_bytes(*k_access.value());
            let meta = v_access.value().to_vec();
            result.push((id, meta));
        }

        Ok(result)
    }

    fn delete(&self, id: Uuid) -> Result<Option<Vec<u8>>, SessionStoreError> {
        let key = id.as_bytes();
        let write_txn = self.db.begin_write()?;
        let buf = {
            let tb_meta = TableMeta::new(&self.meta_table_name);
            let tb_data = TableData::new(&self.data_table_name);
            let mut tb_meta = write_txn.open_table(tb_meta)?;
            let mut tb_data = write_txn.open_table(tb_data)?;

            tb_meta.remove(key)?;
            tb_data.remove(key)?.map(|v| v.value().to_vec())
        };
        write_txn.commit()?;
        Ok(buf)
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
        let write_txn = self.db.begin_write()?;
        let (new_meta_bytes, new_data_bytes) = {
            let tb_meta = TableMeta::new(&self.meta_table_name);
            let tb_data = TableData::new(&self.data_table_name);
            let mut tb_data = write_txn.open_table(tb_data)?;
            let mut tb_meta = write_txn.open_table(tb_meta)?;

            let old_data = tb_data.get(key)?.map(|v| v.value().to_vec());
            let old_meta = tb_meta.get(key)?.map(|v| v.value().to_vec());

            let (new_meta, new_data) = f(old_meta, old_data)?;

            tb_data.insert(key, new_data.as_slice())?;
            tb_meta.insert(key, new_meta.as_slice())?;

            (new_meta, new_data)
        };
        write_txn.commit()?;
        Ok((new_meta_bytes, new_data_bytes))
    }
}
