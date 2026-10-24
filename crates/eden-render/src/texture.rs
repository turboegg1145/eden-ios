//! Byte-bounded texture cache.

use std::sync::Arc;

use eden_paz::cache::WeightedLru;
use eden_paz::image::RgbaImage;

/// Decoded RGBA texture cache with a hard byte ceiling (default 256 MiB).
pub struct TextureCache {
    lru: WeightedLru<String, Arc<RgbaImage>>,
}

impl TextureCache {
    pub fn new(capacity_bytes: usize) -> Self {
        Self {
            lru: WeightedLru::new(capacity_bytes),
        }
    }

    pub fn get(&mut self, name: &str) -> Option<Arc<RgbaImage>> {
        self.lru.get(&name.to_string()).cloned()
    }

    pub fn insert(&mut self, name: impl Into<String>, image: Arc<RgbaImage>) -> bool {
        let weight = image.data.len();
        self.lru.insert(name.into(), image, weight)
    }

    pub fn get_or_insert_with<F>(&mut self, name: &str, f: F) -> Arc<RgbaImage>
    where
        F: FnOnce() -> Arc<RgbaImage>,
    {
        if let Some(img) = self.get(name) {
            return img;
        }
        let img = f();
        self.insert(name, img.clone());
        img
    }

    pub fn used(&self) -> usize {
        self.lru.used()
    }

    pub fn len(&self) -> usize {
        self.lru.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lru.is_empty()
    }

    pub fn clear(&mut self) {
        self.lru.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32) -> Arc<RgbaImage> {
        Arc::new(RgbaImage::filled(w, h, [1, 2, 3, 255]))
    }

    #[test]
    fn evicts_by_bytes() {
        let mut c = TextureCache::new(100);
        c.insert("a", img(5, 5)); // 100 bytes
        assert_eq!(c.used(), 100);
        c.insert("b", img(5, 5)); // evicts a
        assert!(c.get("a").is_none());
        assert!(c.get("b").is_some());
        assert_eq!(c.used(), 100);
    }

    #[test]
    fn get_or_insert() {
        let mut c = TextureCache::new(1000);
        let a = c.get_or_insert_with("x", || img(2, 2));
        let b = c.get_or_insert_with("x", || panic!("must not reload"));
        assert_eq!(a, b);
    }
}
