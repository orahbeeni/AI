//! Incremental commit-graph lane layout.
//!
//! Commits are fed in topological order (children before parents). Each call
//! to [`GraphLayout::push`] produces the line segments needed to draw one row,
//! so layout cost is O(lanes) per commit and can run while history streams in.

use git2::Oid;

/// Which half of the row a segment occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegKind {
    /// Lane continues straight through the row (top edge to bottom edge).
    Pass,
    /// From the top edge of the row into the node (a child joins here).
    In,
    /// From the node down to the bottom edge (towards a parent).
    Out,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seg {
    pub kind: SegKind,
    /// Column at the top edge (`Pass`/`In`) or the node column (`Out`).
    pub from: u16,
    /// Column at the bottom edge (`Out`) or the node column (`In`).
    pub to: u16,
    pub color: u8,
}

#[derive(Clone, Debug, Default)]
pub struct GraphRow {
    pub node_col: u16,
    pub node_color: u8,
    pub segs: Vec<Seg>,
}

#[derive(Clone, Copy)]
struct Lane {
    expects: Oid,
    color: u8,
}

pub const PALETTE_SIZE: u8 = 8;

#[derive(Default)]
pub struct GraphLayout {
    lanes: Vec<Option<Lane>>,
    next_color: u8,
    /// Widest the graph has been so far (in columns).
    pub max_cols: u16,
}

impl GraphLayout {
    pub fn new() -> Self {
        Self::default()
    }

    fn new_color(&mut self) -> u8 {
        let c = self.next_color;
        self.next_color = (self.next_color + 1) % PALETTE_SIZE;
        c
    }

    fn free_slot(&mut self) -> usize {
        match self.lanes.iter().position(Option::is_none) {
            Some(i) => i,
            None => {
                self.lanes.push(None);
                self.lanes.len() - 1
            }
        }
    }

    pub fn push(&mut self, id: Oid, parents: &[Oid]) -> GraphRow {
        let mut segs = Vec::with_capacity(self.lanes.len() + 2);

        // Lanes that were waiting for this commit.
        let first_match = self
            .lanes
            .iter()
            .position(|l| matches!(l, Some(l) if l.expects == id));
        let (col, color) = match first_match {
            Some(i) => (i, self.lanes[i].unwrap().color),
            None => {
                let c = self.new_color();
                (self.free_slot(), c)
            }
        };

        for (i, lane) in self.lanes.iter_mut().enumerate() {
            match *lane {
                Some(l) if l.expects == id => {
                    segs.push(Seg { kind: SegKind::In, from: i as u16, to: col as u16, color: l.color });
                    *lane = None;
                }
                Some(l) => {
                    segs.push(Seg { kind: SegKind::Pass, from: i as u16, to: i as u16, color: l.color });
                }
                None => {}
            }
        }

        for (n, p) in parents.iter().enumerate() {
            let existing = self
                .lanes
                .iter()
                .position(|l| matches!(l, Some(l) if l.expects == *p));
            if let Some(j) = existing {
                let c = self.lanes[j].unwrap().color;
                segs.push(Seg { kind: SegKind::Out, from: col as u16, to: j as u16, color: c });
            } else if n == 0 {
                self.lanes[col] = Some(Lane { expects: *p, color });
                segs.push(Seg { kind: SegKind::Out, from: col as u16, to: col as u16, color });
            } else {
                let c = self.new_color();
                let j = self.free_slot();
                self.lanes[j] = Some(Lane { expects: *p, color: c });
                segs.push(Seg { kind: SegKind::Out, from: col as u16, to: j as u16, color: c });
            }
        }

        while matches!(self.lanes.last(), Some(None)) {
            self.lanes.pop();
        }
        let width = segs
            .iter()
            .map(|s| s.from.max(s.to) + 1)
            .max()
            .unwrap_or(0)
            .max(col as u16 + 1);
        self.max_cols = self.max_cols.max(width);

        GraphRow { node_col: col as u16, node_color: color, segs }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(n: u8) -> Oid {
        let mut b = [0u8; 20];
        b[0] = n;
        Oid::from_bytes(&b).unwrap()
    }

    #[test]
    fn linear_history_uses_one_column() {
        let mut g = GraphLayout::new();
        let rows = [
            g.push(oid(3), &[oid(2)]),
            g.push(oid(2), &[oid(1)]),
            g.push(oid(1), &[]),
        ];
        assert!(rows.iter().all(|r| r.node_col == 0));
        assert_eq!(g.max_cols, 1);
    }

    #[test]
    fn merge_opens_and_closes_a_lane() {
        let mut g = GraphLayout::new();
        // 4 = merge(2, 3); 3 and 2 both child of 1
        let r4 = g.push(oid(4), &[oid(2), oid(3)]);
        let r3 = g.push(oid(3), &[oid(1)]);
        let r2 = g.push(oid(2), &[oid(1)]);
        let r1 = g.push(oid(1), &[]);
        assert_eq!(r4.node_col, 0);
        assert_eq!(r3.node_col, 1);
        assert_eq!(r2.node_col, 0);
        // 2 hands its lane over to 3's lane, where 1 then lands.
        assert_eq!(r1.node_col, 1);
        assert_eq!(r1.segs.iter().filter(|s| s.kind == SegKind::In).count(), 1);
        assert_eq!(g.max_cols, 2);
    }
}
