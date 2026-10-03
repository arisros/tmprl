//! Tracks: one strip of an area cut into fixed and weighted spans, as a dashboard's rows are.

use crate::{Axis, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Track {
    Weight(u16),
    Cells(u16),
}

/// Cut `area` along `axis`. Fixed tracks get their cells, weighted ones share the rest, and
/// none is smaller than `min`.
///
/// When they cannot all fit, tracks are dropped from the end, so the result may be shorter
/// than `tracks`. A track that would be too small to read is worse than one that is absent
/// and counted. The first track is never dropped: an area too small even for it is returned
/// whole.
pub fn tracks(area: Rect, axis: Axis, tracks: &[Track], min: u16) -> Vec<Rect> {
    if area.is_empty() || tracks.is_empty() {
        return Vec::new();
    }
    let extent = area.extent(axis) as u32;
    let floor = |t: &Track| match t {
        Track::Cells(n) => (*n).max(min) as u32,
        Track::Weight(_) => min as u32,
    };
    let mut kept = tracks.len();
    while kept > 1 && tracks[..kept].iter().map(floor).sum::<u32>() > extent {
        kept -= 1;
    }
    let tracks = &tracks[..kept];
    let floors: u32 = tracks.iter().map(floor).sum();
    if floors > extent {
        return vec![area];
    }

    let spare = extent - floors;
    let weights: u32 = tracks
        .iter()
        .map(|t| match t {
            Track::Weight(w) => (*w).max(1) as u32,
            Track::Cells(_) => 0,
        })
        .sum();
    let mut spans: Vec<u32> = tracks
        .iter()
        .map(|t| match t {
            Track::Weight(w) => floor(t) + (*w).max(1) as u32 * spare / weights,
            Track::Cells(_) => floor(t),
        })
        .collect();

    if weights > 0 {
        let mut leftover = extent - spans.iter().sum::<u32>();
        for (span, track) in spans.iter_mut().zip(tracks) {
            if leftover == 0 {
                break;
            }
            if matches!(track, Track::Weight(_)) {
                *span += 1;
                leftover -= 1;
            }
        }
    }

    let mut at = match axis {
        Axis::Columns => area.x,
        Axis::Rows => area.y,
    };
    spans
        .into_iter()
        .map(|span| {
            let span = span as u16;
            let rect = match axis {
                Axis::Columns => Rect::new(at, area.y, span, area.height),
                Axis::Rows => Rect::new(area.x, at, area.width, span),
            };
            at += span;
            rect
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heights(area: Rect, list: &[Track], min: u16) -> Vec<u16> {
        tracks(area, Axis::Rows, list, min)
            .iter()
            .map(|r| r.height)
            .collect()
    }

    #[test]
    fn weights_share_what_the_fixed_tracks_leave() {
        let area = Rect::new(0, 2, 80, 33);
        let list = [Track::Cells(3), Track::Weight(3), Track::Weight(2)];
        let rects = tracks(area, Axis::Rows, &list, 3);
        assert_eq!(heights(area, &list, 3), [3, 18, 12]);
        assert_eq!(rects[0], Rect::new(0, 2, 80, 3));
        assert_eq!(rects[2].bottom(), area.bottom(), "no row left unpainted");
    }

    #[test]
    fn cells_left_over_by_the_division_go_to_the_earliest_weights() {
        let area = Rect::new(0, 0, 80, 20);
        let list = [
            Track::Weight(1),
            Track::Cells(4),
            Track::Weight(1),
            Track::Weight(1),
        ];
        assert_eq!(heights(area, &list, 3), [6, 4, 5, 5]);
    }

    #[test]
    fn columns_are_cut_along_x() {
        let area = Rect::new(4, 1, 90, 10);
        let rects = tracks(
            area,
            Axis::Columns,
            &[Track::Weight(2), Track::Weight(1)],
            24,
        );
        assert_eq!(rects[0], Rect::new(4, 1, 52, 10));
        assert_eq!(rects[1], Rect::new(56, 1, 38, 10));
    }

    #[test]
    fn a_light_track_still_gets_its_minimum() {
        let area = Rect::new(0, 0, 60, 10);
        let rects = tracks(
            area,
            Axis::Columns,
            &[Track::Weight(100), Track::Weight(1)],
            24,
        );
        assert!(rects.iter().all(|r| r.width >= 24), "{rects:?}");
        assert_eq!(rects.iter().map(|r| r.width).sum::<u16>(), 60);
    }

    #[test]
    fn tracks_that_do_not_fit_are_dropped_from_the_end() {
        let area = Rect::new(0, 0, 60, 10);
        let three = [Track::Weight(1); 3];
        let rects = tracks(area, Axis::Columns, &three, 24);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects.iter().map(|r| r.width).sum::<u16>(), 60);
        assert_eq!(
            heights(
                area,
                &[Track::Cells(3), Track::Cells(3), Track::Cells(6)],
                3
            ),
            [3, 3]
        );
    }

    #[test]
    fn an_area_too_small_for_one_track_is_returned_whole() {
        let area = Rect::new(3, 3, 10, 2);
        assert_eq!(tracks(area, Axis::Rows, &[Track::Weight(1); 2], 3), [area]);
        assert_eq!(tracks(area, Axis::Rows, &[Track::Cells(8)], 3), [area]);
    }

    #[test]
    fn nothing_comes_of_no_tracks_or_no_area() {
        assert!(tracks(Rect::new(0, 0, 10, 10), Axis::Rows, &[], 3).is_empty());
        assert!(tracks(Rect::new(0, 0, 0, 10), Axis::Rows, &[Track::Weight(1)], 3).is_empty());
    }
}
