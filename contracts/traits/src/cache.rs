//! Closes #811: TTL cache for cross-contract reads.
//! Starter single-entry cache; a full `Mapping<K, V>`-backed multi-key
//! version with lending/bridge adoption is a follow-up.

/// Caches a single value for `ttl` ticks (block numbers), avoiding a
/// repeated cross-contract read within that window.
pub struct TtlCache<V: Clone> {
    value: Option<V>,
    cached_at: u32,
    ttl: u32,
}

impl<V: Clone> TtlCache<V> {
    pub fn new(ttl: u32) -> Self {
        Self {
            value: None,
            cached_at: 0,
            ttl,
        }
    }

    /// Returns the cached value if still fresh at `now`, else `None`.
    pub fn get(&self, now: u32) -> Option<V> {
        match &self.value {
            Some(v) if now.saturating_sub(self.cached_at) < self.ttl => Some(v.clone()),
            _ => None,
        }
    }

    pub fn set(&mut self, value: V, now: u32) {
        self.value = Some(value);
        self.cached_at = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_value_within_ttl() {
        let mut cache = TtlCache::new(10);
        cache.set(42u32, 100);
        assert_eq!(cache.get(105), Some(42));
    }

    #[test]
    fn expires_after_ttl() {
        let mut cache: TtlCache<u32> = TtlCache::new(10);
        cache.set(42, 100);
        assert_eq!(cache.get(111), None);
    }

    #[test]
    fn entry_is_stale_exactly_at_the_ttl_boundary() {
        let mut cache: TtlCache<u32> = TtlCache::new(10);
        cache.set(42, 100);
        // now - cached_at == ttl is already stale; only strictly-fresher reads hit.
        assert_eq!(cache.get(110), None);
        assert_eq!(cache.get(109), Some(42));
    }

    #[test]
    fn fresh_writes_reset_the_expiry_clock() {
        let mut cache: TtlCache<u32> = TtlCache::new(10);
        cache.set(1, 100);
        assert_eq!(cache.get(108), Some(1));
        cache.set(2, 108);
        assert_eq!(cache.get(117), Some(2));
        assert_eq!(cache.get(119), None);
    }
}
