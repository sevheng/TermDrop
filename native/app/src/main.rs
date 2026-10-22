use std::rc::Rc;
use std::sync::Arc;

use termdrop_gpui::backend::{FakeBackend, PlaceholderContent};

fn main() {
    // No real backend is wired yet: fictional hosts, in-memory settings and a
    // placeholder in every tab (see backend.rs).
    termdrop_gpui::run(Arc::new(FakeBackend::new()), Rc::new(PlaceholderContent));
}
