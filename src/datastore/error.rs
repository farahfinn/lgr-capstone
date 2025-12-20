pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("key not found in index during compaction: {0}")]
    CompactionKeyNotFound(String),
    #[error("Failed to acquire mutex lock: a thread panicked while holding the lock.")]
    MutexLockError, // I will add more specific errors here later
}
