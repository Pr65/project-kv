//! Major storage engine of Project-KV, with data persistence support
//!
//! The key of this storage system fixed 8 bytes, while the value is fixed 256 bytes.
//! (it is possible to change the length of value, but impossible to change the length of key due
//! to the internal encoding mechanism)
//!
//! Setting up a `KVStorage` requires an existing `std::File`
//! ```no_run
//!     use std::fs::File;
//!     use kvsys::kvstorage::KVStorage;
//!     // ...
//!     let f = File::create("data.kv").unwrap();
//!     let kv = KVStorage::new(f);
//!     // ...
//! ```
//!
//! While setting up a `KVStorage` engine from existing file even requires opening the same file
//! twice, once for loading existing data, once for appending
//! ```no_run
//!     use std::fs::File;
//!     use std::fs::OpenOptions;
//!     use kvsys::kvstorage::KVStorage;
//!     // ...
//!     let content;
//!     let kv;
//!     {
//!         let f = File::open("data.kv").unwrap();
//!         content = KVStorage::read_log_file(f).unwrap();
//!     }
//!     {
//!         let f = OpenOptions::new().write(true).append(true).open("data.kv").unwrap();
//!         kv = KVStorage::with_content(content, f);
//!     }
//!     // ...
//! ```
//!
//! This API looks ugly, but let us keep it for sometime.

pub mod disklog;

use std::collections::BTreeMap;
use std::fs::File;
use std::ops::Bound::{Included, Excluded};
use std::error::Error;
use std::fmt;
use std::fmt::{Debug, Display, Formatter};
use std::sync::Arc;
use std::u64;
use crate::kvstorage::disklog::{DiskLogWriter, DiskLogReader, DiskLogMessage};

pub const KEY_SIZE: usize = 8;
pub const VALUE_SIZE: usize = 256;

/// `Key` of storage engine
#[derive(Copy, Clone)]
pub struct Key {
    pub data: [u8; KEY_SIZE]
}

/// `Value` of storage engine
#[derive(Copy, Clone)]
pub struct Value {
    pub data: [u8; VALUE_SIZE]
}

impl Debug for Key {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        write!(f, "KEY [")?;
        for byte in self.data.iter() {
            write!(f, "{:02x}", byte)?;
        }
        write!(f, "]")?;
        Ok(())
    }
}

impl Debug for Value {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        write!(f, "VALUE [")?;
        for byte in self.data.iter().take(8) {
            write!(f, "{:02x}", byte)?;
        }
        write!(f, "..]")?;
        Ok(())
    }
}

impl Display for Key {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        write!(f, "{:?} ('{}')", self, String::from_utf8_lossy(&self.data))
    }
}

impl Display for Value {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        write!(f, "{:?} ('{}...')", self, String::from_utf8_lossy(&self.data[0..8]))
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        for (byte1, byte2) in self.data.iter().zip(other.data.iter()) {
            if byte1 != byte2 {
                return false
            }
        }
        true
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        for (byte1, byte2) in self.data.iter().zip(other.data.iter()) {
            if byte1 != byte2 {
                return false
            }
        }
        true
    }
}

impl Eq for Key {
}

impl Eq for Value {
}

impl Key {
    /// Construct a `Key` from a slice. Panics if length of the given slice is not `KEY_SIZE`
    pub fn from_slice(slice: &[u8]) -> Self {
        assert_eq!(slice.len(), KEY_SIZE);
        let mut ret = [0; KEY_SIZE];
        ret.copy_from_slice(slice);
        Key { data: ret }
    }

    /// Construct a `Key` from a slice. Returns `None` if length of the given slice is not
    /// `KEY_SIZE`
    pub fn from_slice_checked(slice: &[u8]) -> Option<Self> {
        if slice.len() != KEY_SIZE {
            None
        } else {
            let mut ret = [0; KEY_SIZE];
            ret.copy_from_slice(slice);
            Some(Key { data: ret })
        }
    }

    /// Serialize a `Key` into a byte buffer
    pub fn serialize(&self) -> Vec<u8> {
        self.data.to_vec()
    }

    /// Encode a `Key` into a single `u64` for comparing and sorting use.
    /// ```no_run
    ///     use kvsys::kvstorage::Key;
    ///     // ...
    ///     let flat = [0x40u8, 0x49, 0x0f, 0xd0, 0xca, 0xfe, 0xba, 0xbe];
    ///     let expected = 0x40490fd0cafebabeu64;
    ///     let encoded = Key::encode_raw(&flat);
    ///     assert_eq!(encoded, expected);
    /// ```
    pub fn encode(&self) -> InternKey {
        unsafe {
            let flat = &self.data as *const u8 as *const u64;
            u64::from_be(*flat)
        }
    }

    /// Encode an array of `KEY_SIZE` bytes into a single `u64`
    pub fn encode_raw(raw: &[u8; KEY_SIZE]) -> InternKey {
        unsafe {
            let flat = raw as *const u8 as *const u64;
            u64::from_be(*flat)
        }
    }

    /// Decode a `u64` and get the original `Key`
    pub fn decode(encoded: InternKey) -> Self {
        unsafe {
            let bytes = &(u64::to_be(encoded)) as *const u64 as *const [u8; 8];
            Key::from_slice(&(*bytes))
        }
    }
}

impl Value {
    /// Construct a `Value` from a slice. Panics if length of the given slice is not `VALUE_SIZE`
    pub fn from_slice(slice: &[u8]) -> Self {
        assert_eq!(slice.len(), VALUE_SIZE);
        let mut ret = [0; VALUE_SIZE];
        ret.copy_from_slice(slice);
        Value { data: ret }
    }

    /// Construct a `Value` from a slice. Returns `None` if length of the given slice is not `VALUE_SIZE`
    pub fn from_slice_checked(slice: &[u8]) -> Option<Self> {
        if slice.len() != VALUE_SIZE {
            None
        } else {
            let mut ret = [0; VALUE_SIZE];
            ret.copy_from_slice(slice);
            Some(Value { data: ret })
        }
    }

    /// Serialize a `Value` into a byte buffer
    pub fn serialize(&self) -> Vec<u8> {
        self.data.to_vec()
    }
}

type InternKey = u64;

/// A Key-Value storage engine
pub struct KVStorage {
    mem_storage: BTreeMap<InternKey, Option<Arc<Value>>>,
    log_writer: disklog::DiskLogWriter
}

impl Debug for KVStorage {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        write!(f, "KV [")?;
        for (key, maybe_value) in self.mem_storage.iter() {
            if let Some(value) = maybe_value {
                write!(f, "{:?} => {:?},", key, value)?;
            }
        }
        write!(f, "]")
    }
}

impl KVStorage {
    /// Create a `KVStorage` using given `log_file` as its log output
    pub fn new(log_file: File) -> Self {
        KVStorage{ mem_storage: BTreeMap::new(), log_writer: DiskLogWriter::new(log_file) }
    }

    /// Reads `log_file` and constructs a memory storage. This API looks bogus, but let us keep it for a while
    pub fn read_log_file(log_file: File) -> Result<BTreeMap<InternKey, Option<Arc<Value>>>, Box<dyn Error>> {
        let mut ret = BTreeMap::new();
        let mut log_reader = DiskLogReader::new(log_file);
        while let Some(log_msg) = log_reader.next_log()? {
            match log_msg {
                DiskLogMessage::Put(key, value) => {
                    ret.insert(key.encode(), Some(value));
                },
                DiskLogMessage::Delete(key) => {
                    ret.remove(&key.encode());
                }
            }
        }
        Ok(ret)
    }

    /// Create a `KVStorage` using given `log_file` as its log output, and with existing data `mem_storage`
    pub fn with_content(mem_storage: BTreeMap<InternKey, Option<Arc<Value>>>, log_file: File) -> Self {
        KVStorage{ mem_storage, log_writer: DiskLogWriter::new(log_file) }
    }

    /// Trying get the value corresponding to the given `key`, returns `None` if not found
    pub fn get(&self, key: &Key) -> Option<Arc<Value>> {
        let encoded_key = key.encode();
        if let Some(maybe_value) = self.mem_storage.get(&encoded_key) {
            (*maybe_value).clone()
        } else {
            None
        }
    }

    /// Trying put the `key` - `value` pair into storage, returns `Err` if the logging file
    /// unexpectedly goes wrong
    pub fn put(&mut self, key: &Key, value: &Value) -> Result<(), Box<dyn Error>>{
        let encoded_key = key.encode();
        let value = Arc::new(*value);
        self.log_writer.write(DiskLogMessage::Put(*key, value.clone()))?;
        self.mem_storage.insert(encoded_key, Some(value));
        Ok(())
    }

    /// Trying delete the `key` from storage, returns the rows affected (deleted or not, exactly)
    /// if succeeded, `Err` if the internal logging system goes wrong
    pub fn delete(&mut self, key: &Key) -> Result<usize, Box<dyn Error>> {
        let encoded_key = key.encode();
        if let Some(maybe_value) = self.mem_storage.get_mut(&encoded_key) {
            self.log_writer.write(DiskLogMessage::Delete(*key))?;
            *maybe_value = None;
            Ok(1)
        } else {
            Ok(0)
        }
    }

    /// Trying scan all kv pairs within interval [`key1`, `key2`), according to dictionary order
    pub fn scan(&self, key1: &Key, key2: &Key) -> Vec<(Key, Arc<Value>)> {
        let (encoded_key1, encoded_key2) = (key1.encode(), key2.encode());
        self.mem_storage.range((Included(encoded_key1), Excluded(encoded_key2)))
            .filter(|x| {
                let (_, v) = x;
                if let Some(_) = v { true } else { false }
            })
            .map(|x| {
                let (k, v) = x;
                (Key::decode(*k), v.as_ref().unwrap().clone())
            })
            .collect::<Vec<_>>()
    }
}

#[cfg(test)]
mod tests {
    use crate::kvstorage::Key;

    #[test]
    fn test_encode_raw() {
        let flat = [0x40u8, 0x49, 0x0f, 0xd0, 0xca, 0xfe, 0xba, 0xbe];
        let expected = 0x40490fd0cafebabeu64;
        let encoded = Key::encode_raw(&flat);
        assert_eq!(encoded, expected);

        let decoded = Key::decode(encoded);
        assert_eq!(decoded, Key::from_slice(&flat));
    }

    #[test]
    fn test_encode() {
        let flat = [0x00u8, 0x00, 0x00, 0x00, 0x00, 0x3c, 0x9a, 0x0e];
        let flat = Key::from_slice(&flat);
        let expected = 0x3c9a0eu64;
        let encoded = flat.encode();
        assert_eq!(encoded, expected);

        let decoded = Key::decode(encoded);
        assert_eq!(decoded, flat);
    }
}
