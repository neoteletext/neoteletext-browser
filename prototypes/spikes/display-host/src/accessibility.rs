//! The window's accessibility tree, published to the user's screen reader by
//! the display host, outside the window host's confinement (decision 32). The
//! confined window host has no accessibility bus; what a screen reader needs
//! comes from state the trusted core holds: the strip's address and each page
//! image's cleaned text (requirement 12.9).

use std::sync::{Arc, Mutex};

use accesskit::{
    Action, ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, Node, NodeId, Rect, Role, TextPosition,
    TextSelection, TreeId, TreeInfo, TreeUpdate,
};
use smithay::reexports::calloop::channel::Sender;

const WINDOW: NodeId = NodeId(0);
const STRIP: NodeId = NodeId(1);
const PAGE: NodeId = NodeId(2);
/// The strip's text: screen readers read a text field through its text runs.
const STRIP_TEXT: NodeId = NodeId(3);
const APPLICATION_NAME: &str = "NeoTeletext";
const STRIP_LABEL: &str = "Address";

/// What the tree is built from.
#[derive(Clone, Default)]
pub struct Snapshot {
    pub strip_text: String,
    pub strip_focused: bool,
    pub page_text: String,
    /// The window's size, and the strip's height, in logical pixels.
    pub width: f64,
    pub height: f64,
    pub strip_height: f64,
}

fn tree(snapshot: &Snapshot) -> TreeUpdate {
    let window_bounds = Rect { x0: 0.0, y0: 0.0, x1: snapshot.width, y1: snapshot.height };
    let strip_bounds = Rect { y1: snapshot.strip_height, ..window_bounds };
    let mut window = Node::new(Role::Window);
    window.set_label(APPLICATION_NAME);
    window.set_bounds(window_bounds);
    window.set_children(vec![STRIP, PAGE]);

    let mut strip = Node::new(Role::TextInput);
    strip.set_label(STRIP_LABEL);
    strip.set_value(snapshot.strip_text.clone());
    strip.set_bounds(strip_bounds);
    strip.add_action(Action::Focus);
    strip.set_children(vec![STRIP_TEXT]);
    let end = TextPosition { node: STRIP_TEXT, character_index: snapshot.strip_text.chars().count() };
    strip.set_text_selection(TextSelection { anchor: end, focus: end });
    let mut text = Node::new(Role::TextRun);
    text.set_value(snapshot.strip_text.clone());
    text.set_bounds(strip_bounds);
    let lengths: Vec<u8> = snapshot.strip_text.chars().map(|character| u8::try_from(character.len_utf8()).unwrap_or(u8::MAX)).collect();
    text.set_character_lengths(lengths);

    let mut page = Node::new(Role::Image);
    page.set_label(snapshot.page_text.clone());
    page.set_bounds(Rect { y0: snapshot.strip_height, ..window_bounds });
    // Focusable, so a screen reader announces the page when the user moves to it.
    page.add_action(Action::Focus);
    let mut tree = TreeInfo::new(WINDOW);
    tree.toolkit_name = Some(APPLICATION_NAME.into());
    TreeUpdate {
        nodes: vec![(WINDOW, window), (STRIP, strip), (STRIP_TEXT, text), (PAGE, page)],
        tree: Some(tree),
        tree_id: TreeId::ROOT,
        focus: if snapshot.strip_focused { STRIP } else { PAGE },
    }
}

/// A request from the screen reader, carried to the event loop.
pub enum Request {
    FocusStrip,
}

struct Activation(Arc<Mutex<Snapshot>>);
impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.0.lock().ok().map(|snapshot| tree(&snapshot))
    }
}

struct Actions(Sender<Request>);
impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        if request.target_node == STRIP && request.action == Action::Focus {
            let _ = self.0.send(Request::FocusStrip);
        }
    }
}

struct Deactivation;
impl DeactivationHandler for Deactivation {
    fn deactivate_accessibility(&mut self) {}
}

pub struct Accessibility {
    adapter: accesskit_unix::Adapter,
    snapshot: Arc<Mutex<Snapshot>>,
}

impl Accessibility {
    pub fn new(page_text: String, requests: Sender<Request>) -> Self {
        let snapshot = Arc::new(Mutex::new(Snapshot { page_text, ..Snapshot::default() }));
        let adapter = accesskit_unix::Adapter::new(Activation(snapshot.clone()), Actions(requests), Deactivation);
        Self { adapter, snapshot }
    }

    pub fn update(&mut self, strip_text: &str, strip_focused: bool, size: (f64, f64), strip_height: f64) {
        let current = {
            let Ok(mut snapshot) = self.snapshot.lock() else { return };
            strip_text.clone_into(&mut snapshot.strip_text);
            snapshot.strip_focused = strip_focused;
            (snapshot.width, snapshot.height) = size;
            snapshot.strip_height = strip_height;
            snapshot.clone()
        };
        self.adapter.update_if_active(|| tree(&current));
    }

    /// Where the window is on the user's screen, outer edge and content.
    pub fn window_bounds(&mut self, outer: Rect, inner: Rect) {
        self.adapter.set_root_window_bounds(outer, inner);
    }

    pub fn window_focused(&mut self, focused: bool) {
        self.adapter.update_window_focus_state(focused);
    }
}
