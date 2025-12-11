use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::{self, BufReader, BufWriter};
use std::path::Path;

/// A priority queue with a fixed capacity limit.
///
/// It maintains elements ordered by priority `P`. Lower values of `P` are considered
/// lower priority, and higher values are higher priority.
/// When the queue is full, the element with the *lowest* priority is evicted.
/// If multiple elements have the same priority, they are handled in FIFO order.
#[derive(Debug, Serialize, Deserialize)]
pub struct PriorityQueue<P, T>
where
    P: Ord,
{
    // BTreeMap keeps keys sorted.
    // P is priority. Min key = Min priority. Max key = Max priority.
    // Value is VecDeque for FIFO at stable priority.
    map: BTreeMap<P, VecDeque<T>>,
    capacity: usize,
    size: usize,
}

impl<P, T> PriorityQueue<P, T>
where
    P: Ord + Clone + Serialize + DeserializeOwned,
    T: Serialize + DeserializeOwned,
{
    /// Creates a new `PriorityQueue` with the specified capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            map: BTreeMap::new(),
            capacity,
            size: 0,
        }
    }

    /// Adds an element to the queue with the given priority.
    ///
    /// If the queue is at capacity, the element with the lowest priority is removed.
    /// This could be the element currently being added if it has the lowest priority.
    pub fn enqueue(&mut self, priority: P, item: T) {
        self.map.entry(priority).or_default().push_back(item);
        self.size += 1;

        if self.size > self.capacity {
            self.drop_lowest_priority();
        }
    }

    /// Removes and returns the element with the highest priority.
    /// If multiple elements have the same highest priority, returns the one added earliest (FIFO).
    pub fn dequeue(&mut self) -> Option<(P, T)> {
        if self.size == 0 {
            return None;
        }

        // Use last_entry to find the highest priority
        let mut entry = self.map.last_entry()?;
        let prio = entry.key().clone();
        let deque = entry.get_mut();
        let item = deque.pop_front()?;

        if deque.is_empty() {
            entry.remove();
        }

        self.size -= 1;
        Some((prio, item))
    }

    /// Returns a reference to the element with the highest priority.
    pub fn peek(&self) -> Option<(&P, &T)> {
        let (prio, deque) = self.map.last_key_value()?;
        let item = deque.front()?;
        Some((prio, item))
    }

    /// Current number of elements in the queue.
    pub fn len(&self) -> usize {
        self.size
    }

    /// Returns true if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Clears the queue.
    pub fn clear(&mut self) {
        self.map.clear();
        self.size = 0;
    }

    /// Saves the queue state to a file (JSON format).
    pub fn save_to_file<Q: AsRef<Path>>(&self, path: Q) -> io::Result<()> {
        let file = File::create(path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer(writer, self)?;
        Ok(())
    }

    /// Restores the queue state from a file.
    pub fn load_from_file<Q: AsRef<Path>>(path: Q) -> io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let queue: Self = serde_json::from_reader(reader)?;
        Ok(queue)
    }

    fn drop_lowest_priority(&mut self) {
        // Drop from the front (min key).
        if let Some(mut entry) = self.map.first_entry() {
            let deque = entry.get_mut();
            deque.pop_front();
            if deque.is_empty() {
                entry.remove();
            }
            self.size -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_enqueue_dequeue_order() {
        let mut pq = PriorityQueue::new(10);
        pq.enqueue(1, "low".to_string());
        pq.enqueue(3, "high".to_string());
        pq.enqueue(2, "mid".to_string());

        assert_eq!(pq.dequeue(), Some((3, "high".to_string())));
        assert_eq!(pq.dequeue(), Some((2, "mid".to_string())));
        assert_eq!(pq.dequeue(), Some((1, "low".to_string())));
        assert_eq!(pq.dequeue(), None);
    }

    #[test]
    fn test_fifo_same_priority() {
        let mut pq = PriorityQueue::new(10);
        pq.enqueue(2, "first".to_string());
        pq.enqueue(2, "second".to_string());
        pq.enqueue(1, "low".to_string());

        assert_eq!(pq.dequeue(), Some((2, "first".to_string())));
        assert_eq!(pq.dequeue(), Some((2, "second".to_string())));
        assert_eq!(pq.dequeue(), Some((1, "low".to_string())));
    }

    #[test]
    fn test_capacity_eviction_low_incoming() {
        let mut pq = PriorityQueue::new(2);
        pq.enqueue(10, "high".to_string());
        pq.enqueue(5, "mid".to_string());
        // Full now: [5, 10]

        // Add 1 (lowest). Should be dropped immediately (added then removed, or not added).
        // Since we insert then check size, it enters map then leaves.
        pq.enqueue(1, "low-dropped".to_string());

        assert_eq!(pq.len(), 2);
        assert_eq!(pq.dequeue(), Some((10, "high".to_string())));
        assert_eq!(pq.dequeue(), Some((5, "mid".to_string())));
        assert_eq!(pq.dequeue(), None);
    }

    #[test]
    fn test_capacity_eviction_mid_incoming() {
        let mut pq = PriorityQueue::new(2);
        pq.enqueue(10, "high".to_string());
        pq.enqueue(1, "low".to_string());
        // Full: [1, 10]

        // Add 5. 1 should be dropped. 5 kept.
        pq.enqueue(5, "mid".to_string());

        assert_eq!(pq.len(), 2);
        assert_eq!(pq.dequeue(), Some((10, "high".to_string())));
        assert_eq!(pq.dequeue(), Some((5, "mid".to_string())));
    }

    #[test]
    fn test_peek() {
        let mut pq = PriorityQueue::new(10);
        pq.enqueue(1, "a".to_string());
        pq.enqueue(2, "b".to_string());

        {
            let (p, v) = pq.peek().unwrap();
            assert_eq!(*p, 2);
            assert_eq!(v, "b");
        }
        assert_eq!(pq.len(), 2); // Should not remove

        pq.dequeue();

        {
            let (p, v) = pq.peek().unwrap();
            assert_eq!(*p, 1);
            assert_eq!(v, "a");
        }
    }

    #[test]
    fn test_persistence() {
        let mut pq = PriorityQueue::new(5);
        pq.enqueue(1, "p1".to_string());
        pq.enqueue(2, "p2".to_string());

        let path = "test_pq_persistence.json";
        pq.save_to_file(path).unwrap();

        let mut pq2 = PriorityQueue::<i32, String>::load_from_file(path).unwrap();
        assert_eq!(pq2.capacity, 5);
        assert_eq!(pq2.len(), 2);
        assert_eq!(pq2.dequeue(), Some((2, "p2".to_string())));
        assert_eq!(pq2.dequeue(), Some((1, "p1".to_string())));

        let _ = fs::remove_file(path);
    }
}
