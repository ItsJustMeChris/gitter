//! Lane assignment for drawing the commit graph.
//!
//! Commits arrive in an order where children always precede their parents
//! (`git log --date-order`). Each lane remembers which commit it is waiting
//! for; a commit takes the leftmost lane that expects it, and hands that lane
//! on to its first parent. Extra parents of merge commits get their own lanes.
//! The builder is incremental so more commits can be appended as the history
//! view pages in.

use super::{Commit, Oid};

/// Vertical anchor within a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub from_lane: u16,
    pub from: Anchor,
    pub to_lane: u16,
    pub to: Anchor,
    pub color: u8,
}

#[derive(Clone, Debug, Default)]
pub struct GraphRow {
    pub lane: u16,
    pub color: u8,
    pub segments: Vec<Segment>,
    /// Number of lanes occupied in this row.
    pub width: u16,
    pub is_merge: bool,
}

#[derive(Clone, Debug)]
struct Lane {
    expects: Oid,
    color: u8,
}

#[derive(Clone, Debug, Default)]
pub struct GraphBuilder {
    lanes: Vec<Option<Lane>>,
    next_color: u8,
}

pub const PALETTE_SIZE: u8 = 10;

impl GraphBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    fn alloc_color(&mut self) -> u8 {
        let c = self.next_color;
        self.next_color = (self.next_color + 1) % PALETTE_SIZE;
        c
    }

    fn free_slot(&mut self) -> usize {
        match self.lanes.iter().position(|l| l.is_none()) {
            Some(i) => i,
            None => {
                self.lanes.push(None);
                self.lanes.len() - 1
            }
        }
    }

    pub fn push(&mut self, commit: &Commit) -> GraphRow {
        let mut segments = Vec::new();

        let incoming: Vec<usize> = self
            .lanes
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.as_ref().filter(|l| l.expects == commit.oid).map(|_| i))
            .collect();

        // Lanes that simply continue through this row.
        for (i, lane) in self.lanes.iter().enumerate() {
            if let Some(lane) = lane {
                if lane.expects != commit.oid {
                    segments.push(Segment { from_lane: i as u16, from: Anchor::Top, to_lane: i as u16, to: Anchor::Bottom, color: lane.color });
                }
            }
        }

        let (col, color) = match incoming.first() {
            Some(&first) => (first, self.lanes[first].as_ref().unwrap().color),
            None => {
                let slot = self.free_slot();
                (slot, self.alloc_color())
            }
        };

        for &i in &incoming {
            let lane_color = self.lanes[i].as_ref().unwrap().color;
            segments.push(Segment { from_lane: i as u16, from: Anchor::Top, to_lane: col as u16, to: Anchor::Middle, color: lane_color });
            self.lanes[i] = None;
        }

        if let Some(first_parent) = commit.parents.first() {
            self.lanes[col] = Some(Lane { expects: first_parent.clone(), color });
            segments.push(Segment { from_lane: col as u16, from: Anchor::Middle, to_lane: col as u16, to: Anchor::Bottom, color });
            for parent in commit.parents.iter().skip(1) {
                let existing = self.lanes.iter().position(|l| l.as_ref().is_some_and(|l| &l.expects == parent));
                let (lane, lane_color) = match existing {
                    Some(k) => (k, self.lanes[k].as_ref().unwrap().color),
                    None => {
                        let k = self.free_slot();
                        let c = self.alloc_color();
                        self.lanes[k] = Some(Lane { expects: parent.clone(), color: c });
                        (k, c)
                    }
                };
                segments.push(Segment { from_lane: col as u16, from: Anchor::Middle, to_lane: lane as u16, to: Anchor::Bottom, color: lane_color });
            }
        }

        while matches!(self.lanes.last(), Some(None)) {
            self.lanes.pop();
        }

        let width = segments
            .iter()
            .map(|s| s.from_lane.max(s.to_lane) + 1)
            .max()
            .unwrap_or(0)
            .max(col as u16 + 1);

        GraphRow { lane: col as u16, color, segments, width, is_merge: commit.parents.len() > 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(oid: &str, parents: &[&str]) -> Commit {
        Commit { oid: oid.into(), parents: parents.iter().map(|s| s.to_string()).collect(), author_name: String::new(), author_email: String::new(), author_time: 0, subject: String::new() }
    }

    #[test]
    fn linear_history_uses_one_lane() {
        let mut g = GraphBuilder::new();
        for row in [c("c", &["b"]), c("b", &["a"]), c("a", &[])].iter().map(|x| g.push(x)) {
            assert_eq!(row.lane, 0);
            assert_eq!(row.width, 1);
        }
    }

    #[test]
    fn merge_opens_and_closes_lane() {
        let mut g = GraphBuilder::new();
        let m = g.push(&c("m", &["a", "b"]));
        assert!(m.is_merge);
        assert_eq!(m.width, 2);
        let b = g.push(&c("b", &["a"]));
        assert_eq!(b.lane, 1);
        let a = g.push(&c("a", &[]));
        assert_eq!(a.lane, 0);
        // Both lanes converge into a.
        assert_eq!(a.segments.iter().filter(|s| s.to == Anchor::Middle).count(), 2);
    }
}
