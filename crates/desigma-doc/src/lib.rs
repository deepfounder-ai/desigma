//! desigma document model.
//!
//! This crate is the contract shared by every other crate: importers produce a
//! [`Document`], the renderer draws it, the app edits it through [`Command`]s.
//! Coordinates are in Figma units (1 unit = 1 CSS px at 100% zoom), y down.

mod board;
mod board_blocks;
mod brush;
mod code;
mod comments;
mod components;
mod dev;
pub mod find;
mod geom;
mod history;
mod library;
mod library_ops;
pub mod motion;
mod node;
mod paint;
mod prototype;
mod session;
mod settings;
mod textmodel;
mod variables;
mod variables_apply;
mod variables_edit;
pub mod vector;
mod vector_fit;

pub use board::*;
pub use board_blocks::*;
pub use brush::*;
pub use code::*;
pub use comments::*;
pub use components::*;
pub use dev::*;
pub use geom::*;
pub use history::*;
pub use library::*;
pub use library_ops::*;
pub use motion::{NodeMotion, TimelineSettings};
pub use node::*;
pub use paint::*;
pub use prototype::*;
pub use session::*;
pub use settings::*;
pub use textmodel::*;
pub use variables::*;
pub use variables_apply::*;
pub use variables_edit::*;
pub use vector::{HandleMirror, VectorNetwork};
pub use vector_fit::{fit_polyline, Anchor};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stable node identifier. Imported Figma ids ("12:34") are kept in
/// [`Node::source_id`]; `NodeId` is our own dense key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

#[derive(Debug, thiserror::Error)]
pub enum DocError {
    #[error("node {0:?} not found")]
    NotFound(NodeId),
    #[error("cannot move node {0:?} into its own subtree")]
    Cycle(NodeId),
    #[error("node {0:?} cannot have children")]
    NotAContainer(NodeId),
    #[error("invalid animation edit: {0}")]
    InvalidMotion(&'static str),
    #[error("invalid component edit: {0}")]
    InvalidComponent(String),
}

/// A whole file: pages at the root, each page a tree of nodes.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Document {
    pub name: String,
    /// Design file or board (whiteboard) file; decides the toolbar and panels.
    #[serde(default)]
    pub kind: DocumentKind,
    pub nodes: BTreeMap<NodeId, Node>,
    /// Page node ids in display order. Each page is a node with [`NodeKind::Page`].
    pub pages: Vec<NodeId>,
    pub styles: BTreeMap<String, Style>,
    /// Display metadata for entries of `styles`, same keys.
    #[serde(default)]
    pub style_meta: BTreeMap<String, StyleMeta>,
    /// Prototype flows (starting points), in display order.
    #[serde(default)]
    pub flows: Vec<Flow>,
    /// Variables (design tokens) by id.
    #[serde(default)]
    pub variables: BTreeMap<String, Variable>,
    /// Variable collections (with their modes) by id.
    #[serde(default)]
    pub variable_collections: BTreeMap<String, VariableCollection>,
    /// Ruler guides per page.
    #[serde(default)]
    pub guides: BTreeMap<NodeId, Vec<Guide>>,
    /// Motion timeline settings per page; pages with default settings have no entry. Use
    /// [`Document::timeline`] to read.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub timelines: BTreeMap<NodeId, TimelineSettings>,
    /// Device used when presenting the prototype.
    #[serde(default)]
    pub prototype_device: PrototypeDevice,
    /// Comment threads on the canvas.
    #[serde(default)]
    pub comments: Vec<CommentThread>,
    /// Published / enabled libraries and local copies of library assets.
    #[serde(default)]
    pub library: LibraryState,
    /// Hidden page holding copies of library components; not in `pages`.
    #[serde(default)]
    pub library_page: Option<NodeId>,
    /// Timer and voting session of a board.
    #[serde(default)]
    pub board_session: BoardSession,
    /// Embedded images keyed by content hash (Figma `imageRef`).
    pub images: BTreeMap<String, Vec<u8>>,
    next_id: u64,
    /// Collaboration id range: when set, ids are `(peer << 40) | n`. Not saved.
    #[serde(skip)]
    id_peer: Option<u64>,
    #[serde(skip)]
    peer_next: u64,
}

/// Bits of a [`NodeId`] below the peer index in a collaboration session.
pub const PEER_ID_BITS: u32 = 40;

impl Document {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), ..Default::default() }
    }

    pub fn alloc_id(&mut self) -> NodeId {
        if let Some(peer) = self.id_peer {
            return self.alloc_peer_id(peer);
        }
        self.next_id += 1;
        NodeId(self.next_id)
    }

    /// Switches id allocation to the range of collaboration peer `peer` (host 0, guests 1+):
    /// new ids are `(peer << 40) | n`, above every id already in that range. Existing ids stay
    /// valid. Without this call allocation is the plain counter.
    pub fn set_id_peer(&mut self, peer: u32) {
        let peer = u64::from(peer);
        // The host keeps counting after ids handed out before the session started.
        self.peer_next = if peer == 0 { self.next_id } else { 0 };
        self.id_peer = Some(peer);
    }

    /// Peer index set by [`Self::set_id_peer`].
    pub fn id_peer(&self) -> Option<u32> {
        self.id_peer.map(|p| p as u32)
    }

    /// An unused key for a new style named `{base} {n}`. Inside a collaboration session the key
    /// carries the peer (`{base} {n} (p{peer})`), so two people creating a style at once never
    /// pick the same key; without one it is the plain `{base} {n}`. Returns the key and `n`.
    pub fn fresh_style_key(&self, base: &str) -> (String, u32) {
        let mut n = 1;
        loop {
            let key = match self.id_peer {
                Some(peer) => format!("{base} {n} (p{peer})"),
                None => format!("{base} {n}"),
            };
            if !self.styles.contains_key(&key) && !self.style_meta.contains_key(&key) {
                return (key, n);
            }
            n += 1;
        }
    }

    fn alloc_peer_id(&mut self, peer: u64) -> NodeId {
        let base = peer << PEER_ID_BITS;
        let top = self.nodes.range(NodeId(base)..NodeId(base + (1 << PEER_ID_BITS))).next_back().map_or(0, |(id, _)| id.0 - base);
        self.peer_next = self.peer_next.max(top) + 1;
        NodeId(base + self.peer_next)
    }

    /// Allocate safely after importing a document whose stored counter may be
    /// stale, preserving previously allocated IDs and reporting exhaustion.
    pub fn try_alloc_id(&mut self) -> Option<NodeId> {
        if let Some(peer) = self.id_peer {
            return (self.peer_next < (1 << PEER_ID_BITS) - 1).then(|| self.alloc_peer_id(peer));
        }
        let largest = self.nodes.keys().next_back().map_or(0, |id| id.0);
        self.next_id = self.next_id.max(largest).checked_add(1)?;
        Some(NodeId(self.next_id))
    }

    /// Ensures future [`Self::alloc_id`] calls return ids above `id`, so
    /// callers can assign deterministic ids themselves.
    pub fn reserve_ids(&mut self, id: NodeId) {
        if let Some(peer) = self.id_peer {
            let base = peer << PEER_ID_BITS;
            if id.0 >= base && id.0 < base + (1 << PEER_ID_BITS) {
                self.peer_next = self.peer_next.max(id.0 - base);
            }
            return;
        }
        self.next_id = self.next_id.max(id.0);
    }

    /// Copy of the document with the embedded image bytes left out (hash keys stay, with empty
    /// bytes). Cheap shadow / snapshot for sync.
    pub fn clone_without_images(&self) -> Self {
        let images = self.images.keys().map(|k| (k.clone(), Vec::new())).collect();
        Self { images, ..self.clone_shell() }
    }

    fn clone_shell(&self) -> Self {
        Self {
            name: self.name.clone(),
            kind: self.kind,
            nodes: self.nodes.clone(),
            pages: self.pages.clone(),
            styles: self.styles.clone(),
            style_meta: self.style_meta.clone(),
            flows: self.flows.clone(),
            variables: self.variables.clone(),
            variable_collections: self.variable_collections.clone(),
            guides: self.guides.clone(),
            timelines: self.timelines.clone(),
            prototype_device: self.prototype_device.clone(),
            comments: self.comments.clone(),
            library: self.library.clone(),
            library_page: self.library_page,
            board_session: self.board_session.clone(),
            images: BTreeMap::new(),
            next_id: self.next_id,
            id_peer: self.id_peer,
            peer_next: self.peer_next,
        }
    }

    /// Display name of a shared style: its metadata name, else the key.
    pub fn style_name<'a>(&'a self, key: &'a str) -> &'a str {
        self.style_meta.get(key).map(|m| m.name.as_str()).filter(|n| !n.is_empty()).unwrap_or(key)
    }

    /// Motion timeline settings of `page` (defaults when never changed).
    pub fn timeline(&self, page: NodeId) -> TimelineSettings {
        self.timelines.get(&page).copied().unwrap_or_default()
    }

    pub fn add_page(&mut self, name: impl Into<String>) -> NodeId {
        let id = self.alloc_id();
        let mut page = Node::new(id, NodeKind::Page { background: Color::rgb(0xF5, 0xF5, 0xF5) });
        page.name = name.into();
        self.nodes.insert(id, page);
        self.pages.push(id);
        id
    }

    /// Inserts `node` as the last (topmost) child of `parent`.
    pub fn insert(&mut self, parent: NodeId, mut node: Node) -> Result<NodeId, DocError> {
        let p = self.nodes.get_mut(&parent).ok_or(DocError::NotFound(parent))?;
        if !p.kind.is_container() {
            return Err(DocError::NotAContainer(parent));
        }
        let id = node.id;
        p.children.push(id);
        node.parent = Some(parent);
        self.next_id = self.next_id.max(id.0);
        self.nodes.insert(id, node);
        Ok(id)
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&id)
    }

    /// Removes `id` and its subtree, returning the removed nodes (root first).
    pub fn remove(&mut self, id: NodeId) -> Result<Vec<Node>, DocError> {
        let parent = self.nodes.get(&id).ok_or(DocError::NotFound(id))?.parent;
        if let Some(p) = parent.and_then(|p| self.nodes.get_mut(&p)) {
            p.children.retain(|c| *c != id);
        }
        self.pages.retain(|p| *p != id);
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            if let Some(node) = self.nodes.remove(&n) {
                stack.extend(node.children.iter().rev().copied());
                out.push(node);
            }
        }
        Ok(out)
    }

    /// Moves `id` under `new_parent` at `index` (clamped).
    pub fn reparent(&mut self, id: NodeId, new_parent: NodeId, index: usize) -> Result<(), DocError> {
        if self.is_ancestor(id, new_parent) || id == new_parent {
            return Err(DocError::Cycle(id));
        }
        if !self.nodes.get(&new_parent).ok_or(DocError::NotFound(new_parent))?.kind.is_container() {
            return Err(DocError::NotAContainer(new_parent));
        }
        let old = self.nodes.get(&id).ok_or(DocError::NotFound(id))?.parent;
        if let Some(p) = old.and_then(|p| self.nodes.get_mut(&p)) {
            p.children.retain(|c| *c != id);
        }
        let p = self.nodes.get_mut(&new_parent).unwrap();
        let index = index.min(p.children.len());
        p.children.insert(index, id);
        self.nodes.get_mut(&id).unwrap().parent = Some(new_parent);
        Ok(())
    }

    pub fn is_ancestor(&self, ancestor: NodeId, mut of: NodeId) -> bool {
        while let Some(p) = self.nodes.get(&of).and_then(|n| n.parent) {
            if p == ancestor {
                return true;
            }
            of = p;
        }
        false
    }

    /// Absolute transform (page space) of a node.
    pub fn world_transform(&self, id: NodeId) -> Affine {
        let mut chain = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(n) = self.nodes.get(&c) else { break };
            chain.push(n.transform);
            cur = n.parent;
        }
        chain.iter().rev().fold(Affine::IDENTITY, |acc, t| acc * *t)
    }

    /// Page that contains `id`.
    pub fn page_of(&self, mut id: NodeId) -> Option<NodeId> {
        loop {
            let n = self.nodes.get(&id)?;
            match n.parent {
                Some(p) => id = p,
                None => return matches!(n.kind, NodeKind::Page { .. }).then_some(id),
            }
        }
    }

    /// Pre-order traversal of a subtree, back-to-front paint order.
    pub fn walk(&self, root: NodeId, mut f: impl FnMut(&Node, usize)) {
        fn rec(doc: &Document, id: NodeId, depth: usize, f: &mut dyn FnMut(&Node, usize)) {
            if let Some(n) = doc.nodes.get(&id) {
                f(n, depth);
                for c in &n.children {
                    rec(doc, *c, depth + 1, f);
                }
            }
        }
        rec(self, root, 0, &mut f);
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    /// Parses a native document. Text written by older versions that emulated lists with
    /// literal bullet and "1. " prefixes is converted to real lists.
    pub fn from_json(s: &str) -> serde_json::Result<Self> {
        let mut doc: Self = serde_json::from_str(s)?;
        doc.migrate_text();
        doc.normalize_motion();
        Ok(doc)
    }

    /// Restores animation invariants (sorted unique keyframes, valid clips) of every node, e.g.
    /// after loading a hand-edited file.
    pub fn normalize_motion(&mut self) {
        for node in self.nodes.values_mut() {
            if !node.motion.is_empty() {
                node.motion.normalize();
            }
        }
        for t in self.timelines.values_mut() {
            *t = t.sanitized();
        }
        self.timelines.retain(|_, t| !t.is_default());
    }

    /// Converts literal list prefixes of every text node to real lists (see [`migrate_literal_lists`]).
    pub fn migrate_text(&mut self) {
        for node in self.nodes.values_mut() {
            if let NodeKind::Text(t) = &mut node.kind {
                migrate_literal_lists(t);
            }
        }
    }
}

/// Which node property a shared style applies to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StyleProperty {
    #[default]
    Fill,
    Stroke,
    Text,
    Effect,
    Grid,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StyleMeta {
    /// Name as shown in the UI, may contain "/" for grouping ("Brand/Primary").
    pub name: String,
    /// Library key (Figma style `key`), for linking to published libraries.
    pub key: Option<String>,
    pub description: String,
    /// Property the style is used for (a `Style::Fill` can be a stroke style).
    pub property: StyleProperty,
}

/// Shared (library) style referenced by nodes via `style_refs`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Style {
    Fill(Vec<Paint>),
    Text(TextStyle),
    Effect(Vec<Effect>),
    Grid(Vec<LayoutGrid>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_reparent_remove() {
        let mut d = Document::new("t");
        let page = d.add_page("Page 1");
        let fid = d.alloc_id();
        let frame = d.insert(page, Node::frame(fid, Rect::new(0.0, 0.0, 100.0, 100.0))).unwrap();
        let rid = d.alloc_id();
        let rect = d.insert(page, Node::rectangle(rid, Rect::new(10.0, 10.0, 20.0, 20.0))).unwrap();
        d.reparent(rect, frame, 0).unwrap();
        assert_eq!(d.get(frame).unwrap().children, vec![rect]);
        assert!(d.reparent(frame, rect, 0).is_err());
        assert_eq!(d.page_of(rect), Some(page));
        let removed = d.remove(frame).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(d.get(rect).is_none());
    }

    #[test]
    fn peer_id_ranges() {
        let mut d = Document::new("t");
        let page = d.add_page("P");
        assert_eq!(page, NodeId(1));
        d.set_id_peer(0);
        assert_eq!(d.alloc_id(), NodeId(2));
        let mut g = d.clone();
        g.set_id_peer(3);
        let a = g.alloc_id();
        assert_eq!(a, NodeId((3 << 40) | 1));
        g.insert(page, Node::rectangle(a, Rect::new(0.0, 0.0, 1.0, 1.0))).unwrap();
        // Inserting a foreign id does not move this peer's counter.
        g.insert(page, Node::rectangle(NodeId((5 << 40) | 9), Rect::new(0.0, 0.0, 1.0, 1.0))).unwrap();
        assert_eq!(g.alloc_id(), NodeId((3 << 40) | 2));
        // Rejoining finds ids already present in the range.
        let mut r = g.clone();
        r.set_id_peer(3);
        assert_eq!(r.try_alloc_id(), Some(NodeId((3 << 40) | 2)));
        r.reserve_ids(NodeId((3 << 40) | 10));
        assert_eq!(r.alloc_id(), NodeId((3 << 40) | 11));
        // Host keeps counting from the plain counter and ignores guest ids.
        d.insert(page, Node::rectangle(NodeId((3 << 40) | 1), Rect::new(0.0, 0.0, 1.0, 1.0))).unwrap();
        assert_eq!(d.alloc_id(), NodeId(3));
        // The range is not serialized.
        assert_eq!(Document::from_json(&g.to_json().unwrap()).unwrap().id_peer(), None);
    }

    #[test]
    fn json_roundtrip() {
        let mut d = Document::new("t");
        let page = d.add_page("P");
        let id = d.alloc_id();
        d.insert(page, Node::rectangle(id, Rect::new(0.0, 0.0, 5.0, 5.0))).unwrap();
        let back = Document::from_json(&d.to_json().unwrap()).unwrap();
        assert_eq!(back.nodes.len(), 2);
    }

    #[test]
    fn component_link_roundtrip_and_default() {
        let mut d = Document::new("t");
        let page = d.add_page("P");
        let id = d.alloc_id();
        let mut n = Node::rectangle(id, Rect::new(0.0, 0.0, 5.0, 5.0));
        n.component.source = Some(NodeId(99));
        n.component.overrides.insert(OverrideField::Fills);
        n.component.bindings.insert(OverrideField::Visible, "Show icon".into());
        d.insert(page, n).unwrap();
        let json = d.to_json().unwrap();
        let back = Document::from_json(&json).unwrap();
        assert_eq!(back.get(id).unwrap().component, d.get(id).unwrap().component);
        // Plain nodes omit the field, and files without it still load.
        assert!(!json.contains("\"component\": {}"));
        let page_json = serde_json::to_string(back.get(page).unwrap()).unwrap();
        assert!(!page_json.contains("\"component\""));
    }
}
