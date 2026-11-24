//! 以識別字串記住封面位元組（移植自 Swift `ArtworkCache.swift`），
//! 讓只帶識別字串的快照能補回先前看過的位元組。
//!
//! 最多保留 `capacity` 筆，丟掉存入最久的那筆。非執行緒安全：請在單一佇列／任務內使用。

use crate::types::ArtworkRef;
use std::collections::HashMap;
use std::sync::Arc;

pub const DEFAULT_CAPACITY: usize = 16;

#[derive(Debug)]
pub struct ArtworkCache {
    capacity: usize,
    entries: HashMap<String, Arc<[u8]>>,
    /// 由最舊到最新存入的識別字串。
    order: Vec<String>,
}

impl Default for ArtworkCache {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl ArtworkCache {
    /// `capacity` 下限為 1。
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// 帶位元組：存入並原樣回傳。不帶位元組：回傳補上快取位元組的 ref（沒有就原樣）。`None` 直接穿過。
    pub fn resolve(&mut self, artwork: Option<ArtworkRef>) -> Option<ArtworkRef> {
        let artwork = artwork?;
        match &artwork.data {
            Some(data) => {
                self.store(data.clone(), &artwork.id);
                Some(artwork)
            }
            None => match self.entries.get(&artwork.id) {
                Some(cached) => Some(ArtworkRef {
                    id: artwork.id,
                    data: Some(cached.clone()),
                }),
                None => Some(artwork),
            },
        }
    }

    fn store(&mut self, data: Arc<[u8]>, id: &str) {
        if self.entries.insert(id.to_owned(), data).is_some() {
            self.order.retain(|existing| existing != id);
        }
        self.order.push(id.to_owned());
        while self.order.len() > self.capacity {
            let oldest = self.order.remove(0);
            self.entries.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_a() -> Arc<[u8]> {
        Arc::from(vec![0x01u8, 0x02, 0x03])
    }

    fn bytes_b() -> Arc<[u8]> {
        Arc::from(vec![0x0Au8, 0x0B])
    }

    fn art(id: &str, data: Option<Arc<[u8]>>) -> Option<ArtworkRef> {
        Some(ArtworkRef {
            id: id.to_string(),
            data,
        })
    }

    fn data_of(r: Option<ArtworkRef>) -> Option<Arc<[u8]>> {
        r.and_then(|r| r.data)
    }

    #[test]
    fn stores_data_and_fills_later_missing_data_by_identifier() {
        let mut cache = ArtworkCache::default();
        let with_data = cache.resolve(art("a", Some(bytes_a())));
        assert_eq!(with_data, art("a", Some(bytes_a())));
        let filled = cache.resolve(art("a", None));
        assert_eq!(filled, art("a", Some(bytes_a())));
    }

    #[test]
    fn unknown_identifier_without_data_stays_empty() {
        let mut cache = ArtworkCache::default();
        cache.resolve(art("a", Some(bytes_a())));
        let result = cache.resolve(art("other", None));
        assert_eq!(result, art("other", None));
    }

    #[test]
    fn new_data_for_same_identifier_replaces_cached_data() {
        let mut cache = ArtworkCache::default();
        cache.resolve(art("a", Some(bytes_a())));
        cache.resolve(art("a", Some(bytes_b())));
        assert_eq!(data_of(cache.resolve(art("a", None))), Some(bytes_b()));
    }

    #[test]
    fn evicts_oldest_beyond_capacity() {
        let mut cache = ArtworkCache::new(2);
        cache.resolve(art("a", Some(bytes_a())));
        cache.resolve(art("b", Some(bytes_b())));
        cache.resolve(art("c", Some(bytes_a())));
        assert_eq!(data_of(cache.resolve(art("a", None))), None);
        assert_eq!(data_of(cache.resolve(art("b", None))), Some(bytes_b()));
        assert_eq!(data_of(cache.resolve(art("c", None))), Some(bytes_a()));
    }

    #[test]
    fn refreshing_an_identifier_makes_it_newest_for_eviction() {
        let mut cache = ArtworkCache::new(2);
        cache.resolve(art("a", Some(bytes_a())));
        cache.resolve(art("b", Some(bytes_b())));
        cache.resolve(art("a", Some(bytes_a())));
        cache.resolve(art("c", Some(bytes_b())));
        assert_eq!(data_of(cache.resolve(art("a", None))), Some(bytes_a()));
        assert_eq!(data_of(cache.resolve(art("b", None))), None);
    }

    #[test]
    fn default_capacity_is_sixteen() {
        assert_eq!(DEFAULT_CAPACITY, 16);
        assert_eq!(ArtworkCache::default().capacity, 16);
    }

    #[test]
    fn zero_capacity_still_keeps_one_entry() {
        let mut cache = ArtworkCache::new(0);
        cache.resolve(art("a", Some(bytes_a())));
        assert_eq!(data_of(cache.resolve(art("a", None))), Some(bytes_a()));

        cache.resolve(art("b", Some(bytes_b())));
        assert_eq!(
            data_of(cache.resolve(art("a", None))),
            None,
            "oldest evicted"
        );
        assert_eq!(data_of(cache.resolve(art("b", None))), Some(bytes_b()));
    }

    #[test]
    fn nil_ref_passes_through() {
        assert_eq!(ArtworkCache::default().resolve(None), None);
    }
}
