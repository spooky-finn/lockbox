//! Presentation-only line comparison; never stores revision contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Equal,
    Removed,
    Added,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Line<'a> {
    pub kind: Kind,
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub text: &'a str,
}

pub(super) fn lines<'a>(before: &'a str, after: &'a str, replace: bool) -> Vec<Line<'a>> {
    let old: Vec<_> = before.split_inclusive('\n').collect();
    let new: Vec<_> = after.split_inclusive('\n').collect();
    let mut result = Vec::new();
    let (mut i, mut j) = (0, 0);
    let mut emit = |kind, i: usize, j: usize| {
        result.push(Line {
            kind,
            old: (kind != Kind::Added).then_some(i + 1),
            new: (kind != Kind::Removed).then_some(j + 1),
            text: if kind == Kind::Added { new[j] } else { old[i] },
        });
    };
    // Hidden changes must still show a removal and addition even when both masks match.
    if replace {
        for i in 0..old.len() {
            emit(Kind::Removed, i, 0);
        }
        for j in 0..new.len() {
            emit(Kind::Added, 0, j);
        }
        return result;
    }
    while i < old.len() && j < new.len() && old[i] == new[j] {
        emit(Kind::Equal, i, j);
        i += 1;
        j += 1;
    }
    let (mut end_old, mut end_new) = (old.len(), new.len());
    while end_old > i && end_new > j && old[end_old - 1] == new[end_new - 1] {
        end_old -= 1;
        end_new -= 1;
    }
    let (start_old, start_new) = (i, j);
    let (height, width) = (end_old - i + 1, end_new - j + 1);
    // Bound memory and quadratic work for very large notes. Prefix/suffix context
    // remains exact; an oversized middle is rendered as a complete replacement.
    if height.saturating_mul(width) <= 262_144 {
        let mut lengths = vec![0u32; height * width];
        for a in (0..height - 1).rev() {
            for b in (0..width - 1).rev() {
                lengths[a * width + b] = if old[start_old + a] == new[start_new + b] {
                    1 + lengths[(a + 1) * width + b + 1]
                } else {
                    lengths[(a + 1) * width + b].max(lengths[a * width + b + 1])
                };
            }
        }
        while i < end_old && j < end_new {
            if old[i] == new[j] {
                emit(Kind::Equal, i, j);
                i += 1;
                j += 1;
            } else if lengths[(i - start_old + 1) * width + j - start_new]
                >= lengths[(i - start_old) * width + j - start_new + 1]
            {
                emit(Kind::Removed, i, j);
                i += 1;
            } else {
                emit(Kind::Added, i, j);
                j += 1;
            }
        }
    }
    while i < end_old {
        emit(Kind::Removed, i, j);
        i += 1;
    }
    while j < end_new {
        emit(Kind::Added, i, j);
        j += 1;
    }
    while i < old.len() {
        emit(Kind::Equal, i, j);
        i += 1;
        j += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_lines_keep_context_and_both_line_numbers() {
        let rows = lines("first\nold\nlast", "first\nnew\nlast", false);
        assert_eq!(
            rows,
            vec![
                Line {
                    kind: Kind::Equal,
                    old: Some(1),
                    new: Some(1),
                    text: "first\n"
                },
                Line {
                    kind: Kind::Removed,
                    old: Some(2),
                    new: None,
                    text: "old\n"
                },
                Line {
                    kind: Kind::Added,
                    old: None,
                    new: Some(2),
                    text: "new\n"
                },
                Line {
                    kind: Kind::Equal,
                    old: Some(3),
                    new: Some(3),
                    text: "last"
                },
            ]
        );
    }

    #[test]
    fn insertions_deletions_and_empty_values() {
        assert!(lines("", "", false).is_empty());
        assert!(
            lines("", "a\n\nb", false)
                .iter()
                .all(|r| r.kind == Kind::Added)
        );
        assert!(
            lines("a\nb", "", false)
                .iter()
                .all(|r| r.kind == Kind::Removed)
        );
        let rows = lines("a\nc", "a\nb\nc", false);
        assert_eq!(rows[1].kind, Kind::Added);
        assert_eq!((rows[2].old, rows[2].new), (Some(2), Some(3)));
    }

    #[test]
    fn equal_masks_still_represent_a_secret_change() {
        let rows = lines("••••••••", "••••••••", true);
        assert_eq!(
            rows.iter().map(|r| r.kind).collect::<Vec<_>>(),
            [Kind::Removed, Kind::Added]
        );
    }

    #[test]
    fn large_replacements_preserve_all_content() {
        let before = format!("start\n{}end", "old\n".repeat(600));
        let after = format!("start\n{}end", "new\n".repeat(600));
        let rows = lines(&before, &after, false);
        let restored_before: String = rows
            .iter()
            .filter(|r| r.kind != Kind::Added)
            .map(|r| r.text)
            .collect();
        let restored_after: String = rows
            .iter()
            .filter(|r| r.kind != Kind::Removed)
            .map(|r| r.text)
            .collect();
        assert_eq!(restored_before, before);
        assert_eq!(restored_after, after);
    }
}
