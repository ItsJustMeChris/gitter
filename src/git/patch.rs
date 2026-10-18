//! Building partial patches for hunk- and line-level staging.
//!
//! To stage a subset of lines we rewrite each hunk so that unselected changes
//! become no-ops, then feed the result to `git apply --cached --recount`.
//! When applying in reverse (unstaging or discarding), unselected additions
//! must stay as context and unselected removals disappear, because the target
//! of a reverse apply is the post-image.

use super::{FileDiff, LineKind};

pub fn build_patch(diff: &FileDiff, selected: impl Fn(usize, usize) -> bool, reverse: bool) -> Option<String> {
    let mut body = String::new();
    let mut shift: i64 = 0;
    let mut all_selected = true;

    for (hi, hunk) in diff.hunks.iter().enumerate() {
        let mut out = String::new();
        let mut old_count = 0u32;
        let mut new_count = 0u32;
        let mut changed = false;
        for (li, line) in hunk.lines.iter().enumerate() {
            let sel = selected(hi, li);
            let emitted = match line.kind {
                LineKind::Context => {
                    old_count += 1;
                    new_count += 1;
                    Some(' ')
                }
                LineKind::Added => {
                    if sel {
                        new_count += 1;
                        changed = true;
                        Some('+')
                    } else {
                        all_selected = false;
                        if reverse {
                            old_count += 1;
                            new_count += 1;
                            Some(' ')
                        } else {
                            None
                        }
                    }
                }
                LineKind::Removed => {
                    if sel {
                        old_count += 1;
                        changed = true;
                        Some('-')
                    } else {
                        all_selected = false;
                        if reverse {
                            None
                        } else {
                            old_count += 1;
                            new_count += 1;
                            Some(' ')
                        }
                    }
                }
            };
            if let Some(prefix) = emitted {
                out.push(prefix);
                out.push_str(&line.content);
                out.push('\n');
                if line.no_newline {
                    out.push_str("\\ No newline at end of file\n");
                }
            }
        }
        if !changed {
            continue;
        }
        let (old_start, new_start) = if reverse {
            let new_start = hunk.new_start as i64;
            (new_start - shift, new_start)
        } else {
            let old_start = hunk.old_start as i64;
            (old_start, old_start + shift)
        };
        shift += new_count as i64 - old_count as i64;
        body.push_str(&format!("@@ -{},{} +{},{} @@\n", old_start.max(0), old_count, new_start.max(0), new_count));
        body.push_str(&out);
    }

    if body.is_empty() {
        return None;
    }

    let mut header = String::new();
    for line in &diff.header {
        // A partially staged deletion is no longer a deletion.
        if diff.deleted && !all_selected && !reverse {
            if line.starts_with("deleted file mode") {
                continue;
            }
            if line == "+++ /dev/null" {
                header.push_str(&format!("+++ b/{}\n", diff.path));
                continue;
            }
        }
        if line.starts_with("diff --git") || line.starts_with("---") || line.starts_with("+++") || line.starts_with("new file mode") || line.starts_with("deleted file mode") || line.starts_with("old mode") || line.starts_with("new mode") {
            header.push_str(line);
            header.push('\n');
        }
    }
    if !header.contains("\n---") && !header.starts_with("---") {
        header.push_str(&format!("--- a/{}\n+++ b/{}\n", diff.old_path.as_deref().unwrap_or(&diff.path), diff.path));
    }
    Some(header + &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::parse_diff;

    const DIFF: &[u8] = b"diff --git a/f.txt b/f.txt\nindex 1..2 100644\n--- a/f.txt\n+++ b/f.txt\n@@ -1,4 +1,4 @@\n a\n-b\n+B\n c\n-d\n+D\n";

    #[test]
    fn select_single_line_forward() {
        let d = &parse_diff(DIFF)[0];
        // Stage only "+B" (hunk 0, line 2) and "-b" (line 1).
        let p = build_patch(d, |_, l| l == 1 || l == 2, false).unwrap();
        assert!(p.contains("@@ -1,4 +1,4 @@\n a\n-b\n+B\n c\n d\n"), "{p}");
    }

    #[test]
    fn select_single_line_reverse() {
        let d = &parse_diff(DIFF)[0];
        // Unstage only "+D" (line 5): "-d" is unselected and dropped, "+B" becomes context.
        let p = build_patch(d, |_, l| l == 5, true).unwrap();
        assert!(p.contains("@@ -1,3 +1,4 @@\n a\n B\n c\n+D\n"), "{p}");
    }

    #[test]
    fn nothing_selected() {
        let d = &parse_diff(DIFF)[0];
        assert!(build_patch(d, |_, _| false, false).is_none());
    }
}
