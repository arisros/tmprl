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
    fn refreshing_keeps_existing_data_on_screen() {
        let mut l = Loadable::loaded(7u8);
        l.begin_refresh();
        assert_eq!(l.value(), Some(&7), "a refresh must not blank the view");

        let mut e: Loadable<u8> = Loadable::Failed("boom".into());
        e.begin_refresh();
        assert!(e.is_loading());
    }
}
