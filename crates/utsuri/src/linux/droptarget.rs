//! Linux: drops on the menu arrive through `linux::dnd` (an invisible Wayland surface over
//! the menu), started once in `main`; nothing to register per window.

pub fn ensure(_title: &str, _app: &rune::AppHandle<crate::Hub>) {}
