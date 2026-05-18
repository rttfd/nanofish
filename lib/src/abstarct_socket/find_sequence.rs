/// Utility to find value sequences in a stream.
pub struct FindSequence<'a, Item> {
    needle: &'a [Item],
    position: usize,
}

impl<'a, Item> FindSequence<'a, Item> {
    /// Create a new FindSequence for the given byte needle sequence
    ///
    /// ## Panics
    /// ### Panics if the provided sequence is empty.
    #[must_use]
    pub fn new(needle: &'a [Item]) -> Self {
        if needle.is_empty() {
            panic!("Empty sequence is not allowed");
        }

        Self { needle, position: 0 }
    }

    /// Push a byte into the sequence finder
    pub fn check_next(&mut self, byte: Item) -> bool
    where
        Item: PartialEq + Copy,
    {
        // Safety: position is always less than sequence length
        if byte != unsafe { *self.needle.get_unchecked(self.position) } {
            // Mismatch found, reset position
            self.position = 0;
            return false;
        }

        self.position += 1;
        if self.position == self.needle.len() {
            // Needle fully matched
            self.position = 0;
            return true;
        }

        false
    }

    /// Treats each next subsequence as a subslice of a single contiguous sequence. Checks bytes one by one on the
    /// the whole sequence and returns Some(index of the subsequence element beyond the last byte of the needle
    /// sequence) if found contiguous subsequence called needle or None otherwise.
    #[must_use]
    pub fn check_next_slice(&mut self, subsequence: &[Item]) -> Option<usize>
    where
        Item: PartialEq + Copy,
    {
        for (i, &byte) in subsequence.iter().enumerate() {
            if self.check_next(byte) {
                return Some(i + 1);
            }
        }
        None
    }
}

/// Tests for FindSequence
#[cfg(all(test, not(feature = "embassy_impl")))]
pub mod tests {
    use super::*;

    #[test]
    fn test_check_next_byte() {
        let sequence = b"\r\n";
        let mut finder = FindSequence::new(sequence);
        let data = b"Hello, World!\r\nThis is a test.\r\n";
        let mut found_positions = Vec::new();
        for (i, &byte) in data.iter().enumerate() {
            if finder.check_next(byte) {
                found_positions.push(i + 1 - sequence.len());
            }
        }
        assert_eq!(found_positions, vec![13, 30]);
    }

    #[test]
    fn test_check_next_slice() {
        let sequence = b"\r\n";
        let mut finder = FindSequence::new(sequence);
        let data_slices = [
            &b"Hello, "[..],
            &b"World!\r"[..],
            &b"\nThis is a test.\r"[..],
            &b"\n"[..],
        ];
        let mut found_positions = Vec::new();
        for slice in &data_slices {
            let mut remaining_slice = *slice;
            while let Some(matched_bytes) = finder.check_next_slice(remaining_slice) {
                found_positions.push(matched_bytes);
                remaining_slice = &remaining_slice[matched_bytes..];
            }
        }
        assert_eq!(found_positions, vec![1, 1]);
    }

    #[test]
    #[should_panic(expected = "Empty sequence is not allowed")]
    fn test_check_next_slice_for_empty_sequence() {
        let empty_sequence = b"";
        let _ = FindSequence::new(empty_sequence);
    }

    #[test]
    fn test_check_next_slice_with_empty_slice_must_return_none() {
        let sequence = b"\r\n";
        let mut finder = FindSequence::new(sequence);
        let result = finder.check_next_slice(b"");
        assert_eq!(result, None);
    }
}
