use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use crate::{datastore::error::DbError, EmbeddedDatabase, Result};

/// A thread-safe wrapper around `EmbeddedDatabase`, enabling concurrent access from multiple threads.
///
/// This struct uses `Arc<Mutex<EmbeddedDatabase>>` to provide interior mutability and shared
/// ownership. `Arc` allows multiple threads to own a pointer to the database, while `Mutex`
/// ensures that only one thread can access the underlying `EmbeddedDatabase` at any given time,
/// preventing data races.
#[derive(Clone)]
pub struct ThreadSafeDB {
    db: Arc<Mutex<EmbeddedDatabase>>,
}

impl ThreadSafeDB {
    /// Creates a new `ThreadSafeDB` instance or opens an existing one from the specified path.
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        // Initialize the single-threaded EmbeddedDatabase
        let db = EmbeddedDatabase::new(path)?;
        Ok(Self {
            // Wrap the database in a Mutex for exclusive access and an Arc for shared ownership.
            db: Arc::new(Mutex::new(db)),
        })
    }

    /// Sets a key-value pair in the database.
    pub fn set(&self, key: &str, val: &str) -> Result<()> {
        // Acquire a lock on the mutex. This blocks until exclusive access is granted.
        // `map_err` is used to transform the `PoisonError` (if a thread holding the lock panicked)
        // into our custom `DbError::MutexLockError`.
        let mut locked_db = self.db.lock().map_err(|_| DbError::MutexLockError)?;
        // Once locked, we can safely call the mutable methods of the underlying database.
        locked_db.set(key, val)
    }

    /// Retrieves the value associated with a given key from the database.
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        // Acquire a lock on the mutex for exclusive access.
        let mut locked_db = self.db.lock().map_err(|_| DbError::MutexLockError)?;
        // Safely call the underlying database's get method.
        locked_db.get(key)
    }

    /// Deletes a key-value pair from the database.
    pub fn delete(&self, key: &str) -> Result<()> {
        // Acquire a lock on the mutex for exclusive access.
        let mut locked_db = self.db.lock().map_err(|_| DbError::MutexLockError)?;
        // Safely call the underlying database's delete method.
        locked_db.delete(key)
    }

    /// Closes the database, performing compaction.
    ///
    /// This operation is thread-safe. It acquires a lock on the database, performs the `close`
    /// operation on the underlying `EmbeddedDatabase`, and then releases the lock.
    /// Note: `close` consumes the `EmbeddedDatabase` internally but this wrapper
    /// keeps the `Arc<Mutex>` alive until all clones are dropped. The compaction
    /// and file replacement happens within the `EmbeddedDatabase::close` call.
    pub fn close(&self) -> Result<()> {
        // Acquire a lock on the mutex for exclusive access.
        let mut locked_db = self.db.lock().map_err(|_| DbError::MutexLockError)?;
        // Safely call the underlying database's close method, which handles compaction.
        locked_db.close()
    }
}

#[cfg(test)]
mod test {
    use std::thread;

    use tempfile::NamedTempFile;

    use crate::ThreadSafeDB;

    #[test]
    fn test_thread_safe_db_concurrency() {
        let temp_file = NamedTempFile::new().expect("we should be able to create a temp file");
        let db_path = temp_file.path();

        // initialize a `ThreadSafeDb instance`
        let db = ThreadSafeDB::new(db_path).expect("we should get a new db instance");

        let mut handles = vec![];
        for i in 0..10 {
            let db_clone = db.clone(); // Clone Arc<Mutex<db>> for each thread
            let handle = thread::spawn(move || {
                for j in 0..100 {
                    let key = format!("Thread id {} key {}", i, j);
                    let val = format!("Thread id {} val {}", i, j);
                    // Perform set operation
                    db_clone.set(&key, &val).expect("ThreadSafeDB set failed");

                    // Perfom get operation & verify
                    let retrieved_val = db_clone.get(&key).expect("Failed to get value using key");

                    assert_eq!(retrieved_val, Some(val));
                }

                // Delete a few keys too per thread
                for j in 0..50 {
                    let key = format!("Thread id {} key {}", i, j);
                    db_clone.delete(&key).expect("ThreadSafeDb delete failed");
                }
            });
            handles.push(handle);
        }

        // Join all threads
        for handle in handles {
            handle.join().expect("Thread panicked");
        }

        // Verify final state from the main thread
        // The main thread also needs to lock the DB to access it
        let final_db = ThreadSafeDB::new(db_path)
            .expect("Faild to reopen ThreadSafeDb for final verification");

        for i in 0..10 {
            for j in 0..100 {
                let key = format!("Thread id {} key {}", i, j);
                let expected_val = if j < 50 {
                    // Keys that were deleted by threads
                    None
                } else {
                    // Keys that should still exists
                    Some(format!("Thread id {} val {}", i, j))
                };

                let retrieved_val = final_db.get(&key).expect("Final verification get failed");
                assert_eq!(
                    retrieved_val, expected_val,
                    "Final state mismatch for key: {}",
                    key
                );
            }
        }

        // close and compact the db
        db.close()
            .expect("Failed to close and compact ThreadSafeDb");

        // Reopen and verify again after closing
        let reopened_db =
            ThreadSafeDB::new(db_path).expect("Failed to reopend ThreadSafeDb after compaction");
        for i in 0..10 {
            for j in 0..100 {
                let key = format!("Thread id {} key {}", i, j);
                let expected_val = if j < 50 {
                    None
                } else {
                    Some(format!("Thread id {} val {}", i, j))
                };

                let retrieved_val = reopened_db
                    .get(&key)
                    .expect("Final verification get failed");
                assert_eq!(
                    retrieved_val, expected_val,
                    "Final state mismatch for key: {}",
                    key
                );
            }
        }
    }
}
