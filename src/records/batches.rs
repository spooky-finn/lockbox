use crate::{records::entry::Entry, serialization::json_size, sync::protocol as sync};

/// Stay well inside one shared transaction (mutation count and encoded size).
pub(crate) const BATCH_MUTATIONS: usize = sync::MAX_MUTATIONS / 2;
const BATCH_BYTES: usize = sync::MAX_TRANSACTION_BYTES / 2;

/// Splits entries so that each shared transaction stays within its limits.
pub(crate) fn entry_batches(entries: &[Entry]) -> impl Iterator<Item = &[Entry]> {
    let mut sizes = entries
        .iter()
        .map(|entry| json_size(entry).unwrap_or(BATCH_BYTES))
        .enumerate()
        .peekable();
    std::iter::from_fn(move || {
        let (start, mut bytes) = sizes.next()?;
        let mut end = start + 1;
        while let Some(&(index, size)) = sizes.peek() {
            if index - start == BATCH_MUTATIONS || bytes + size > BATCH_BYTES {
                break;
            }
            let _ = sizes.next();
            bytes += size;
            end = index + 1;
        }
        Some(&entries[start..end])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batching_preserves_empty_oversized_and_exact_count_boundaries() {
        let lengths = |entries: &[Entry]| {
            entry_batches(entries)
                .map(|batch| batch.len())
                .collect::<Vec<_>>()
        };
        assert!(lengths(&[]).is_empty());
        let mut oversized = Entry::new();
        oversized.notes = "x".repeat(BATCH_BYTES + 1);
        let entries = [Entry::new(), oversized, Entry::new()];
        assert_eq!(lengths(&entries), [1, 1, 1]);
        assert_eq!(
            entry_batches(&entries).collect::<Vec<_>>().concat(),
            entries
        );

        let entries: Vec<_> = (0..BATCH_MUTATIONS + 1).map(|_| Entry::new()).collect();
        assert_eq!(lengths(&entries), [BATCH_MUTATIONS, 1]);
    }

    #[test]
    fn large_imports_are_split_into_bounded_batches() {
        let mut entries = Vec::new();

        for index in 0..BATCH_MUTATIONS + 3 {
            let mut entry = Entry::new();
            entry.title = format!("entry {index}");
            entries.push(entry);
        }

        let mut big = Entry::new();
        big.title = "big".into();
        big.add_attachment("blob".into(), vec![7; 900 * 1024])
            .unwrap();
        entries.insert(1, big.clone());
        entries.insert(2, big);

        let batches: Vec<_> = entry_batches(&entries).collect();
        assert_eq!(
            batches.iter().map(|b| b.len()).sum::<usize>(),
            entries.len()
        );
        assert!(batches.iter().all(|b| b.len() <= BATCH_MUTATIONS));
        assert!(batches.iter().all(|b| {
            b.len() == 1
                || b.iter()
                    .map(|e| serde_json::to_vec(e).unwrap().len())
                    .sum::<usize>()
                    <= BATCH_BYTES
        }));
        assert!(batches.len() >= 3);
    }

    #[test]
    fn batch_byte_boundaries_match_json_escaping_unicode_and_attachments() {
        let mut first = Entry::new();
        first.title = "Quoted \"title\" and backslash \\".into();
        first
            .add_attachment("ключ.bin".into(), vec![0, 255, 42])
            .unwrap();
        let mut second = Entry::new();
        second.title = "Unicode 🗝️".into();
        second.username = "名\n\t\"\\".into();
        let mut third = Entry::new();
        third.title = "Third".into();
        let encoded_len = |entry: &Entry| serde_json::to_vec(entry).unwrap().len();
        let padding = BATCH_BYTES - encoded_len(&first) - encoded_len(&second);
        first.notes = "a".repeat(padding / 2);
        second.notes = "b".repeat(padding - padding / 2);
        assert_eq!(encoded_len(&first) + encoded_len(&second), BATCH_BYTES);
        let mut entries = [first, second, third];

        assert_eq!(
            entry_batches(&entries)
                .map(|batch| batch.len())
                .collect::<Vec<_>>(),
            [2, 1]
        );
        entries[1].notes.push('x');
        assert_eq!(
            encoded_len(&entries[0]) + encoded_len(&entries[1]),
            BATCH_BYTES + 1
        );
        assert_eq!(
            entry_batches(&entries)
                .map(|batch| batch.len())
                .collect::<Vec<_>>(),
            [1, 2]
        );
    }
}
