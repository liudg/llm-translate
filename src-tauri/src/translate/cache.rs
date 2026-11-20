//! In-memory LRU cache: (source, direction, model) -> translation, 500 entries.

use std::num::NonZeroUsize;
use std::sync::Mutex;

use lru::LruCache;

use super::detect::Direction;

type Key = (String, Direction, String);

pub struct Cache(Mutex<LruCache<Key, String>>);

impl Cache {
    pub fn new(cap: usize) -> Self {
        Self(Mutex::new(LruCache::new(NonZeroUsize::new(cap).unwrap())))
    }

    pub fn get(&self, src: &str, dir: Direction, model: &str) -> Option<String> {
        self.0.lock().unwrap().get(&(src.to_string(), dir, model.to_string())).cloned()
    }

    pub fn put(&self, src: &str, dir: Direction, model: &str, out: String) {
        self.0.lock().unwrap().put((src.to_string(), dir, model.to_string()), out);
    }
}
