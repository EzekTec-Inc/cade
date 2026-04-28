//! UI extension slots — typed insertion points for plugin-injected
//! widgets in the CADE TUI layout.
//!
//! ## Why
//!
//! Today extensibility is limited to two ad-hoc fields: `header_lines:
//! Vec<RenderLine>` and `footer_extra: Option<String>`.  Any new
//! injection point requires touching `TuiApp` and the render path.
//!
//! This module provides a typed slot manager so plugins (and future
//! built-in features) can occupy named regions — `Header`, `Footer`,
//! `Sidebar`, etc. — without each one negotiating a private field.
//!
//! ## Contract
//!
//! - **`UiSlot`** enumerates the well-known regions.
//! - **`SlotComponent`** is the host-agnostic widget interface.
//! - **`SlotManager`** is the registry.  `set` installs a widget,
//!   `take` removes it, `get_mut` accesses the live instance during
//!   render.
//!
//! Adoption is incremental: the existing `header_lines` /
//! `footer_extra` paths continue to work, and only new code needs to
//! go through the slot manager.

use std::collections::HashMap;

use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect};

use crate::colors::ThemeColors;

/// Well-known UI regions where a [`SlotComponent`] can be installed.
///
/// Additional variants may be added in future milestones (e.g.
/// `StatusBar`, `Toast`).  Keep the enum non-exhaustive so adding
/// regions does not break match arms in plugins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UiSlot {
    /// Top region — typically a status bar or breadcrumb.
    Header,
    /// Bottom region — typically a hint line or progress indicator.
    Footer,
    /// Right-hand sidebar — typically a tree or contextual panel.
    Sidebar,
}

/// Host-agnostic widget interface for slot-installed components.
///
/// Distinct from [`crate::component::Component`] (which returns
/// `Vec<RenderedLine>`) because slot widgets draw directly into a
/// [`ratatui::Frame`], the same surface the rest of the TUI uses.
pub trait SlotComponent {
    /// Render the widget into `area` of `frame`.
    fn render(&mut self, frame: &mut Frame, area: Rect, colors: &ThemeColors);

    /// Optionally consume keyboard input that arrived while this slot
    /// has focus.  The default returns `false` (passive widget).
    fn handle_input(&mut self, _key: KeyEvent) -> bool {
        false
    }

    /// Preferred height in rows.  The host may grant fewer.  `0`
    /// means "use whatever is left".
    fn preferred_height(&self) -> u16 {
        1
    }
}

/// Registry of [`SlotComponent`]s keyed by [`UiSlot`].
///
/// One instance lives on `TuiApp`; the render path queries it for
/// each known slot.  Zero or one component per slot — installing a
/// new one displaces the previous occupant (the host should call
/// [`Self::take`] first if it needs the displaced widget back).
#[derive(Default)]
pub struct SlotManager {
    slots: HashMap<UiSlot, Box<dyn SlotComponent>>,
}

impl SlotManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install `component` in `slot`, returning the previous
    /// occupant if any.
    pub fn set(
        &mut self,
        slot: UiSlot,
        component: Box<dyn SlotComponent>,
    ) -> Option<Box<dyn SlotComponent>> {
        self.slots.insert(slot, component)
    }

    /// Remove and return the component currently in `slot`, if any.
    pub fn take(&mut self, slot: UiSlot) -> Option<Box<dyn SlotComponent>> {
        self.slots.remove(&slot)
    }

    /// Borrow the component in `slot` for rendering.
    pub fn get_mut(&mut self, slot: UiSlot) -> Option<&mut Box<dyn SlotComponent>> {
        self.slots.get_mut(&slot)
    }

    /// `true` when `slot` currently holds a component.
    pub fn is_occupied(&self, slot: UiSlot) -> bool {
        self.slots.contains_key(&slot)
    }

    /// Number of installed slots — useful for capacity planning in
    /// the layout step.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal stub used to verify the registry surface.
    struct StubSlot {
        height: u16,
        rendered: bool,
    }

    impl SlotComponent for StubSlot {
        fn render(&mut self, _f: &mut Frame, _a: Rect, _c: &ThemeColors) {
            self.rendered = true;
        }
        fn preferred_height(&self) -> u16 {
            self.height
        }
    }

    #[test]
    fn slot_manager_starts_empty() {
        let m = SlotManager::new();
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
        assert!(!m.is_occupied(UiSlot::Header));
    }

    #[test]
    fn set_then_get_returns_installed_widget() {
        let mut m = SlotManager::new();
        let stub = StubSlot {
            height: 3,
            rendered: false,
        };
        let prev = m.set(UiSlot::Header, Box::new(stub));
        assert!(prev.is_none());
        assert!(m.is_occupied(UiSlot::Header));
        let got = m.get_mut(UiSlot::Header);
        assert!(got.is_some());
        assert_eq!(got.unwrap().preferred_height(), 3);
    }

    #[test]
    fn set_displaces_previous_occupant() {
        let mut m = SlotManager::new();
        m.set(
            UiSlot::Footer,
            Box::new(StubSlot {
                height: 1,
                rendered: false,
            }),
        );
        let prev = m.set(
            UiSlot::Footer,
            Box::new(StubSlot {
                height: 2,
                rendered: false,
            }),
        );
        // Old occupant returned to caller.
        assert!(prev.is_some());
        // Footer now holds the new one.
        assert_eq!(
            m.get_mut(UiSlot::Footer).unwrap().preferred_height(),
            2
        );
    }

    #[test]
    fn take_removes_and_returns_widget() {
        let mut m = SlotManager::new();
        m.set(
            UiSlot::Sidebar,
            Box::new(StubSlot {
                height: 5,
                rendered: false,
            }),
        );
        let taken = m.take(UiSlot::Sidebar);
        assert!(taken.is_some());
        assert!(!m.is_occupied(UiSlot::Sidebar));
    }

    #[test]
    fn slots_are_independent() {
        let mut m = SlotManager::new();
        m.set(
            UiSlot::Header,
            Box::new(StubSlot {
                height: 1,
                rendered: false,
            }),
        );
        m.set(
            UiSlot::Footer,
            Box::new(StubSlot {
                height: 1,
                rendered: false,
            }),
        );
        assert_eq!(m.len(), 2);
        m.take(UiSlot::Header);
        assert_eq!(m.len(), 1);
        assert!(m.is_occupied(UiSlot::Footer));
    }

    #[test]
    fn slot_component_default_handle_input_is_passive() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut s = StubSlot {
            height: 1,
            rendered: false,
        };
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(!s.handle_input(key));
    }

    #[test]
    fn ui_slot_is_hashable_for_use_as_map_key() {
        let mut m = std::collections::HashMap::new();
        m.insert(UiSlot::Header, "h");
        m.insert(UiSlot::Footer, "f");
        m.insert(UiSlot::Sidebar, "s");
        assert_eq!(m.len(), 3);
    }
}
