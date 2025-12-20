use std::thread;
use tiny_db_exp::{Result, ThreadSafeDB}; // Assuming crate is named tiny_db_exp, I might change this lateer

/// Using my custom `Result` type to avoid handling errors here.
/// Only for the demo.
fn main() -> Result<()> {
    println!("--- Tiny DB Experience Demo ---");

    // Use a temporary directory for this demo run
    let db_path = std::env::temp_dir().join("tiny-db-demo.db");
    println!("Database file will be created at: {:?}\n", db_path);

    // Clean up any previous demo database file
    if db_path.exists() {
        std::fs::remove_file(&db_path)?;
    }

    // --- Part 1: Basic Single-Threaded Operations ---
    println!("--- Running Part 1: Basic Operations ---");
    {
        let db = ThreadSafeDB::new(&db_path)?;
        println!("Setting key 'name' to 'Alice'");
        db.set("name", "Alice")?;

        println!("Getting key 'name'...");
        let value = db.get("name")?;
        println!("  -> Retrieved: {:?}\n", value);
        assert_eq!(value, Some("Alice".to_string()));

        println!("Updating key 'name' to 'Bob'");
        db.set("name", "Bob")?;

        println!("Getting key 'name' again...");
        let value = db.get("name")?;
        println!("  -> Retrieved: {:?}\n", value);
        assert_eq!(value, Some("Bob".to_string()));

        println!("Deleting key 'name'");
        db.delete("name")?;

        println!("Getting key 'name' one last time...");
        let value = db.get("name")?;
        println!("  -> Retrieved: {:?}\n", value);
        assert_eq!(value, None);
    } // db is dropped here, but file remains

    // --- Part 2: Multi-Threaded Concurrent Access ---
    println!("--- Running Part 2: Concurrent Operations ---");
    let db = ThreadSafeDB::new(&db_path)?;
    let mut handles = vec![];
    let num_threads = 5;

    println!("Spawning {} threads to write concurrently...", num_threads);

    for i in 0..num_threads {
        let db_clone = db.clone();
        let handle = thread::spawn(move || {
            let key = format!("thread_key_{}", i);
            let value = format!("thread_value_{}", i);
            println!("  [Thread {}] Setting key '{}' to '{}'", i, key, value);
            db_clone.set(&key, &value).unwrap();
        });
        handles.push(handle);
    }

    // Wait for all threads to finish
    for handle in handles {
        handle.join().unwrap();
    }

    println!("\nAll threads finished. Verifying data from main thread:");

    for i in 0..num_threads {
        let key = format!("thread_key_{}", i);
        let expected_value = Some(format!("thread_value_{}", i));
        let retrieved_value = db.get(&key)?;
        println!(
            "  Verifying key '{}': Retrieved {:?}, Expected {:?}",
            key, retrieved_value, expected_value
        );
        assert_eq!(retrieved_value, expected_value);
    }

    // --- Part 3: Compaction on Close ---
    println!("\n--- Running Part 3: Closing and Compaction ---");
    let initial_size = std::fs::metadata(&db_path)?.len();
    println!("Size before close (compaction): {} bytes", initial_size);

    // This will trigger the compaction logic
    db.close()?;

    let final_size = std::fs::metadata(&db_path)?.len();
    println!("Size after close (compaction):  {} bytes", final_size);
    assert!(final_size < initial_size);
    println!("Compaction successful! File size reduced.");

    println!("\n--- Demo Complete ---");

    Ok(())
}
