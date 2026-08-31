//! Four-state remote data.
//!
//! Every value fetched over the network is one of these. Views render all four, which is
//! how the interface avoids ever having a code path that blocks waiting for data: there is
//! no way to express "waiting", only "not here yet", which draws a skeleton.

use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub enum Loadable<T> {
    #[default]
    NotAsked,
    Loading,
    Loaded(T, Instant),
    Failed(String),
}

impl<T> Loadable<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Loadable::Loaded(v, _) => Some(v),
            _ => None,
        }
    }

    /// Mutable access to loaded data, for a list that grows a page at a time rather than
    /// being replaced wholesale.
    pub fn value_mut(&mut self) -> Option<&mut T> {
        match self {
            Loadable::Loaded(v, _) => Some(v),
            _ => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Loadable::Loading)
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Loadable::Failed(e) => Some(e),
            _ => None,
        }
    }

    /// How stale the data is, for the statusline.
    pub fn age(&self) -> Option<std::time::Duration> {
        match self {
            Loadable::Loaded(_, at) => Some(at.elapsed()),
            _ => None,
        }
    }

    pub fn loaded(value: T) -> Self {
        Loadable::Loaded(value, Instant::now())
    }

    /// Mark as loading while keeping any value already on screen, so a refresh does not
    /// blank the view.
    pub fn begin_refresh(&mut self) {
        if !matches!(self, Loadable::Loaded(..)) {
            *self = Loadable::Loading;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessors_report_the_right_state() {
        let n: Loadable<u8> = Loadable::NotAsked;
        assert!(n.value().is_none() && n.error().is_none() && !n.is_loading());

        let l = Loadable::loaded(7u8);
        assert_eq!(l.value(), Some(&7));
        assert!(l.age().is_some());

        let f: Loadable<u8> = Loadable::Failed("boom".into());
        assert_eq!(f.error(), Some("boom"));
    }

    #[test]
    fn loaded_data_can_be_grown_in_place() {
        // Infinite scroll appends to a list that is already on screen; replacing the
        // Loadable would drop the fetch time the statusline reports staleness from.
        let mut l = Loadable::loaded(vec![1u8]);
        let at = l.age();
        l.value_mut().unwrap().push(2);
        assert_eq!(l.value(), Some(&vec![1, 2]));
        assert!(at.is_some() && l.age().is_some());

        let mut n: Loadable<Vec<u8>> = Loadable::NotAsked;
        assert!(n.value_mut().is_none());
    }

    #[test]
    fn refreshing_keeps_existing_data_on_screen() {
        let mut l = Loadable::loaded(7u8);
        l.begin_refresh();
        assert_eq!(l.value(), Some(&7), "a refresh must not blank the view");

        let mut e: Loadable<u8> = Loadable::Failed("boom".into());
        e.begin_refresh();
        assert!(e.is_loading());
    }
}
