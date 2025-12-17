# On-Disk Data Structure Diagram

This document illustrates the append-only log format used by the embedded database.
The database file is a sequence of records. Each record is prefixed with its length, stored as an 8-byte unsigned integer (`u64`).

---

### Structure of a Single File Entry

Each entry written to the data file consists of two parts:

```
+--------------------------------+--------------------------------------+
|          8-byte Header         |                 Data                 |
+--------------------------------+--------------------------------------+
| Length of the serialized Record|  The bincode-serialized Record struct|
| (as u64, little-endian)        |                                      |
+--------------------------------+--------------------------------------+
```

---

### Example Scenario

Let's trace the file layout for the following sequence of operations:

1.  `db.set("name", "Alice")`
2.  `db.set("city", "Berlin")`
3.  `db.delete("name")`

The raw data file would look like this, with each new record appended to the end:

#### File Layout

```
================================== FILE START ==================================

[Record 1: A call to `set("name", "Alice")`]
+----------------------+----------------------------------------------------------+
| Bytes 0-7: Length    | Bytes 8-18: Record Data                                  |
+----------------------+----------------------------------------------------------+
| 0x0B00000000000000   | Bincode representation of the Record struct below        |
| (Length = 11 bytes*)|                                                          |
+----------------------+----------------------------------------------------------+
  └─ Corresponds to ──> Record { key: "name", val: "Alice" }


[Record 2: A call to `set("city", "Berlin")`]
+----------------------+----------------------------------------------------------+
| Bytes 19-26: Length  | Bytes 27-38: Record Data                                 |
+----------------------+----------------------------------------------------------+
| 0x0C00000000000000   | Bincode representation of the Record struct below        |
| (Length = 12 bytes*)|                                                          |
+----------------------+----------------------------------------------------------+
  └─ Corresponds to ──> Record { key: "city", val: "Berlin" }


[Record 3: A call to `delete("name")`, which creates a tombstone]
+----------------------+----------------------------------------------------------+
| Bytes 39-46: Length  | Bytes 47-52: Record Data                                 |
+----------------------+----------------------------------------------------------+
| 0x0600000000000000   | Bincode representation of the Record struct below        |
| (Length = 6 bytes*)  |                                                          |
+----------------------+----------------------------------------------------------+
  └─ Corresponds to ──> Record { key: "name", val: "" }


=================================== FILE END ===================================
```

_*Note on lengths: The actual byte length of the serialized data depends on the `bincode` serialization format. The lengths shown here (11, 12, 6) are estimates. `bincode` encodes the length of each string within the struct, plus the string contents themselves. For example, `Record { key: "name", val: "Alice" }` serializes to something conceptually like `[length_of_key: 4]"name"[length_of_value: 5]"Alice"`._

---

### Index Reconstruction

When the database is started (`EmbeddedDatabase::new`), it reads this file from start to finish to rebuild the in-memory index:

1.  **Reads Record 1**: It sees `{ key: "name", val: "Alice" }`. It adds `"name"` to the index, pointing to the start of this record (byte 0).
    *   `index` is now `{ "name": 0 }`
2.  **Reads Record 2**: It sees `{ key: "city", val: "Berlin" }`. It adds `"city"` to the index, pointing to the start of this record (byte 19).
    *   `index` is now `{ "name": 0, "city": 19 }`
3.  **Reads Record 3**: It sees the tombstone record `{ key: "name", val: "" }`. Because the value is empty, it **removes** `"name"` from the index.
    *   `index` is now `{ "city": 19 }`

The final in-memory index accurately reflects the live, non-deleted data. The old record for `"name"` at byte 0 still exists on disk but is now "dead" space, as it is no longer referenced by the index.
