use crate::datastore::error::DbError;

use super::{Record, Result};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
};
/// The main datastore struct.
/// It holds a file handle to the data file & an in-memory index
pub struct EmbeddedDatabase {
    file: File,
    index: HashMap<String, u64>, // Maps key to byte offset in the file
    path: PathBuf,               // Used to store the path to the db file
}

impl EmbeddedDatabase {
    /// Creates a new EmbeddedDatabase or opens an existing one from a db file
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref(); // Convert generic P into a concrete &Path

        let mut file = OpenOptions::new()
            .read(true) // Allow reading
            .write(true) // Allow writing
            .create(true) // Create if it does not exist
            .truncate(false)
            .open(path)?;
        let mut index = HashMap::new();
        let mut position = 0;
        let file_len = file.metadata()?.len();

        // Read the file & populate the index
        while position < file_len {
            // Move cursor to the begining of the next record
            file.seek(std::io::SeekFrom::Start(position))?;

            // Read the 8-byte length of the serialized record
            // if we can't read the length, we have reached the end of the file
            let mut len_buffer = [0u8; 8];
            if file.read_exact(&mut len_buffer).is_err() {
                break;
            }
            let len = u64::from_le_bytes(len_buffer);

            // Read the record data
            let mut record_buffer = vec![0u8; len as usize];
            file.read_exact(&mut record_buffer)?;

            let record: Record = bincode::deserialize(&record_buffer)?;

            // Check if the record is a tombstone
            if record.val.is_empty() {
                // Remove the key from the index
                index.remove(&record.key);
            } else {
                // The start of the record is the curent "position"
                index.insert(record.key, position);
            };

            position += 8 + len;
        }

        Ok(EmbeddedDatabase {
            file,
            index,
            path: path.to_path_buf(),
        })
    }
    /// Serialize a K, V pair and append it to the data file as well as update
    /// in memory idx in order to find the data later without scanning the file.
    /// Our on-disk format for a single entry will look like this :
    /// [8-byte len of record] [actual Record data bytes]
    pub fn set(&mut self, key: &str, val: &str) -> Result<()> {
        // Create a Record with the given key & Val
        let record = Record {
            key: key.to_string(),
            val: val.to_string(),
        };
        /*
        Note to self:
        bincode doesn't just blindly join the bytes of
        the key and value strings. It's more clever than that. Following serde's data
        model, it encodes information about the struct's fields.
        A simplified view of what bincode generates for
        Record {key: "cat",
                value: "meow"
        }
        might look like this:
        [length of key: 3] [actual bytes for "cat"] [length of value: 4] [actual bytes for "meow"]
        */
        let encoded_record = bincode::serialize(&record)?;
        let encoded_record_len = encoded_record.len();

        // Find EOF to to get where to write
        let end_of_file = self.file.seek(std::io::SeekFrom::End(0))?;

        // Add the length of the current record being inserted into the file
        self.file.write_all(&encoded_record_len.to_le_bytes())?;

        // Write the actual contents of the record
        self.file.write_all(&encoded_record)?;

        // Update the in-memory idx
        self.index.insert(key.to_string(), end_of_file); // end_of_file may not be a good variale name here.

        Ok(())
    }
    /// Use in-memory idx to perform a fast lookup
    pub fn get(&mut self, key: &str) -> Result<Option<String>> {
        // Look up requested key in the index HashMap.
        let byte_offset = match self.index.get(key) {
            // get the byte offset of where the record starts in the file
            Some(val) => val,
            // Key does not exist return immediately
            None => return Ok(None),
        };
        // Seek to that exact offset in the file
        let _position_in_file = self.file.seek(std::io::SeekFrom::Start(*byte_offset))?;

        // Read the 8-byte lenght of the serialized record
        let mut buffer_for_length_of_record = [0u8; 8];
        self.file.read_exact(&mut buffer_for_length_of_record)?;
        let len_of_record = u64::from_le_bytes(buffer_for_length_of_record);

        // self.file.read_exact_at(buffer, position_in_file);

        // Convert that buffer of bytes back into the Record struct
        let mut buffer_for_actual_record = vec![0u8; len_of_record as usize];
        self.file.read_exact(&mut buffer_for_actual_record)?;
        let record: Record = bincode::deserialize(&buffer_for_actual_record)?;

        Ok(Some(record.val))
    }
    /// Deletes a record from the live database
    pub fn delete(&mut self, key: &str) -> Result<()> {
        // Create a tombstone record with an empty value
        let record = Record {
            key: key.to_string(),
            val: "".to_string(),
        };

        let encoded_record = bincode::serialize(&record)?;
        let encoded_record_len = encoded_record.len();

        // Go to file end & add the length of tombstone and the empty record
        self.file.seek(std::io::SeekFrom::End(0))?;
        self.file.write_all(&encoded_record_len.to_le_bytes())?;
        self.file.write_all(&encoded_record)?;

        // Also remove the key from the live in memory index
        self.index.remove(key);

        Ok(())
    }

    /// Closes the database, performing compaction.
    pub fn close(&mut self) -> Result<()> {
        // Create a path for the new compacted file.
        // For a db at "my.db", this might be "my.db.compact"
        let mut compact_file_path = self.path.clone();
        compact_file_path.add_extension("compact");

        // Open the new file
        let mut compact_db_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&compact_file_path)?;

        // Create a new idx for the compacted data
        let mut compact_index = HashMap::new();

        // Initialize a counter for the new file's record positions
        let mut new_position: u64 = 0;

        // Note: We need to clone the keys, because we are borrowing `self` mutably
        // later inside the loop when we call `self.file.seek`
        let keys: Vec<String> = self.index.keys().cloned().collect();

        for key in keys {
            // Get the position of the live record from the old idx
            let position = self
                .index
                .get(&key)
                .ok_or_else(|| DbError::CompactionKeyNotFound(key.clone()))?;

            // Seek to that record in the old file
            self.file.seek(std::io::SeekFrom::Start(*position))?;

            // Read the 8-byte length of the record
            let mut record_len_buffer = [0u8; 8];
            self.file.read_exact(&mut record_len_buffer)?;
            let len_of_record = u64::from_le_bytes(record_len_buffer);

            // Read the full record data
            let mut record_buffer = vec![0u8; len_of_record as usize];
            self.file.read_exact(&mut record_buffer)?;

            // Write the length & the record data to the new compacted file
            compact_db_file.write_all(&record_len_buffer)?;
            compact_db_file.write_all(&record_buffer)?;

            // Update the new idx with the key and its new position
            compact_index.insert(key, new_position);

            new_position += 8 + len_of_record;
        }
        // Atomically replace the old database file with the new, compacted one.
        std::fs::rename(&compact_file_path, &self.path)?;

        // Reopen the file handle to point to the new db file.
        // We now want to read & write to the main path
        self.file = OpenOptions::new().read(true).write(true).open(&self.path)?;

        // Update the new in memory idx to be the compacted_index
        self.index = compact_index;

        Ok(())
    }
}

#[cfg(test)]
mod test {

    use super::*;
    use tempfile::NamedTempFile;
    #[test]
    fn test_new_set_and_get() {
        // vreate a temp file for the database
        let temp_file = NamedTempFile::new().expect("we should be able to create a temp file");
        let db_path = temp_file.path();

        let mut db = EmbeddedDatabase::new(db_path)
            .expect("should be able to create a new database in db_path");
        db.set("Name", "Alice")
            .expect("should be able to set a key value pair ");
        let result = db.get("Name").expect("Database should return a record");

        assert_eq!(result, Some("Alice".to_string()));

        // drop db to ensure the file handle is closed & data is flushed
        // this simulates a program restart
        drop(db);

        // Reopen the database, this closes the file handle & flushes any buffered wirtes
        let mut db1 = EmbeddedDatabase::new(db_path)
            .expect("should be able to open the temp db_path a second time");
        let result1 = db1
            .get("Name")
            .expect("to get the value of the key we set previously");

        assert_eq!(result1, Some("Alice".to_string()));

        // Ensure a non-existent key returns none
        let non_existent = db1.get("Age").expect("get to return a none");
        assert_eq!(non_existent, None);
    }
    #[test]
    fn test_delete_persistence() {
        let temp_file = NamedTempFile::new().expect("failed to create temp file");
        let db_path = temp_file.path();
        // Create a db & set a value
        let mut db =
            EmbeddedDatabase::new(db_path).expect(" creating a db using the temp file path failed");
        db.set("Name", "Alice").expect("Failed to create a record");
        assert_eq!(db.get("Name").unwrap(), Some("Alice".to_string()));

        // Delete the value
        db.delete("Name").expect("record deletion failed");
        assert_eq!(db.get("Name").unwrap(), None);

        // Re-open the database to check for perisistence
        drop(db);
        let mut db =
            EmbeddedDatabase::new(db_path).expect(" creating a db using the temp file path failed");
        assert_eq!(
            db.get("Name").unwrap(),
            None,
            "The key should still be deleted after reopening "
        );
    }

    #[test]
    fn test_compaction() {
        let temp_file = NamedTempFile::new().expect("failed to create a temp file");
        let db_path = temp_file.path();

        // Create a db & set multiple values
        let mut db =
            EmbeddedDatabase::new(db_path).expect(" creating a db using the temp file path failed");
        db.set("Name1", "Alice").expect("Failed to create a record");
        db.set("Name2", "Bob").expect("failed to create a record");
        db.set("Name3", "Joe").expect("failed to set a record");

        // Update some records
        db.set("Name1", "Janet").unwrap();
        db.set("Name3", "Finn").unwrap();

        // Delet some keys, creating tombstone records
        db.delete("Name2").unwrap();

        // Add another key after deletion
        db.set("Name4", "Adam").unwrap();

        // Verify initial state before compaction (including dead records)
        assert_eq!(db.get("Name1").unwrap(), Some("Janet".to_string()));
        assert_eq!(db.get("Name2").unwrap(), None);
        assert_eq!(db.get("Name3").unwrap(), Some("Finn".to_string()));
        assert_eq!(db.get("Name4").unwrap(), Some("Adam".to_string()));

        // Store initial file size for comparison
        let initial_db_file_size = db.file.metadata().unwrap().len();

        // Perform compaction
        db.close().expect("failed to close and compact");

        // Verify that the all llive keys are still accessible & hold correct values
        assert_eq!(db.get("Name1").unwrap(), Some("Janet".to_string()));
        assert_eq!(db.get("Name2").unwrap(), None);
        assert_eq!(db.get("Name3").unwrap(), Some("Finn".to_string()));
        assert_eq!(db.get("Name4").unwrap(), Some("Adam".to_string()));

        // Verify that deleted keys are no longer present

        // Verify that file size has decreased
        let new_db_file_size = std::fs::metadata(db.path.clone()).unwrap().len();
        assert!(initial_db_file_size > new_db_file_size);

        // Reopen the database to verify persistence after compaction
        drop(db);

        let mut db =
            EmbeddedDatabase::new(db_path).expect(" creating a db using the temp file path failed");

        // Verify that the data is still accessible & hold correct values
        assert_eq!(db.get("Name1").unwrap(), Some("Janet".to_string()));
        assert_eq!(db.get("Name2").unwrap(), None);
        assert_eq!(db.get("Name3").unwrap(), Some("Finn".to_string()));
        assert_eq!(db.get("Name4").unwrap(), Some("Adam".to_string()));
    }
}
