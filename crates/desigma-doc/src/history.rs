//! Undo/redo. Every edit goes through [`Command`]; applying returns its inverse.

use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Command {
    /// Insert subtree nodes (root first) under `parent` at `index`.
    Insert {
        parent: NodeId,
        index: usize,
        nodes: Vec<Node>,
    },
    Remove {
        id: NodeId,
    },
    Reparent {
        id: NodeId,
        parent: NodeId,
        index: usize,
    },
    /// Replace a node's own properties (children/parent are preserved).
    Update {
        node: Box<Node>,
    },
    /// Insert a page subtree (page node first) at `index` in `Document::pages`.
    InsertPage {
        index: usize,
        nodes: Vec<Node>,
    },
    RemovePage {
        id: NodeId,
    },
    /// Replace the document's prototype flow list (flow starting points).
    SetFlows {
        flows: Vec<Flow>,
    },
    /// Replace shared styles and their metadata (style create / edit / delete).
    SetStyles {
        styles: BTreeMap<String, Style>,
        style_meta: BTreeMap<String, StyleMeta>,
    },
    /// Replace all comment threads.
    SetComments {
        comments: Vec<CommentThread>,
    },
    /// Replace library bookkeeping (publish info, enabled libraries, asset copies).
    SetLibraryState {
        state: LibraryState,
        library_page: Option<NodeId>,
    },
    /// Replace the guides of one page.
    SetGuides {
        page: NodeId,
        guides: Vec<Guide>,
    },
    /// Replace the prototype device settings.
    SetPrototypeDevice {
        device: PrototypeDevice,
    },
    /// Replace all variables and collections (variable edits).
    SetVariables {
        variables: BTreeMap<String, Variable>,
        collections: BTreeMap<String, VariableCollection>,
    },
    /// Replace the board's timer and voting session.
    SetBoardSession {
        session: BoardSession,
    },
    /// Replace one node's animation data (keyframe tracks and clips). All motion edits in
    /// [`crate::motion::edit`] compile to this so undo is a snapshot swap.
    SetNodeMotion {
        node: NodeId,
        motion: Box<NodeMotion>,
    },
    /// Replace the Motion timeline settings of a page.
    SetTimeline {
        page: NodeId,
        settings: TimelineSettings,
    },
    Batch(Vec<Command>),
}

impl Command {
    /// Applies the command and returns the command that undoes it.
    pub fn apply(self, doc: &mut Document) -> Result<Command, DocError> {
        Ok(match self {
            Command::Insert { parent, index, nodes } => {
                let root = nodes.first().map(|n| n.id).ok_or(DocError::NotFound(parent))?;
                let p = doc.get_mut(parent).ok_or(DocError::NotFound(parent))?;
                if !p.kind.is_container() {
                    return Err(DocError::NotAContainer(parent));
                }
                let index = index.min(p.children.len());
                p.children.insert(index, root);
                for mut n in nodes {
                    if n.id == root {
                        n.parent = Some(parent);
                    }
                    doc.nodes.insert(n.id, n);
                }
                doc.bump_ids();
                Command::Remove { id: root }
            }
            Command::Remove { id } => {
                let parent = doc.get(id).ok_or(DocError::NotFound(id))?.parent.ok_or(DocError::NotFound(id))?;
                let index = doc.get(parent).map(|p| p.children.iter().position(|c| *c == id).unwrap_or(0)).unwrap_or(0);
                let nodes = doc.remove(id)?;
                Command::Insert { parent, index, nodes }
            }
            Command::Reparent { id, parent, index } => {
                let n = doc.get(id).ok_or(DocError::NotFound(id))?;
                let old_parent = n.parent.ok_or(DocError::NotFound(id))?;
                let old_index = doc.get(old_parent).and_then(|p| p.children.iter().position(|c| *c == id)).unwrap_or(0);
                doc.reparent(id, parent, index)?;
                Command::Reparent { id, parent: old_parent, index: old_index }
            }
            Command::Update { mut node } => {
                let cur = doc.get_mut(node.id).ok_or(DocError::NotFound(node.id))?;
                node.parent = cur.parent;
                node.children = std::mem::take(&mut cur.children);
                let old = std::mem::replace(cur, *node);
                Command::Update { node: Box::new(Node { children: Vec::new(), ..old }) }
            }
            Command::InsertPage { index, nodes } => {
                let root = nodes.first().ok_or(DocError::NotFound(NodeId(0)))?;
                if !matches!(root.kind, NodeKind::Page { .. }) {
                    return Err(DocError::NotAContainer(root.id));
                }
                let root = root.id;
                for mut n in nodes {
                    if n.id == root {
                        n.parent = None;
                    }
                    doc.nodes.insert(n.id, n);
                }
                let index = index.min(doc.pages.len());
                doc.pages.insert(index, root);
                doc.bump_ids();
                Command::RemovePage { id: root }
            }
            Command::RemovePage { id } => {
                let index = doc.pages.iter().position(|p| *p == id).ok_or(DocError::NotFound(id))?;
                let nodes = doc.remove(id)?;
                Command::InsertPage { index, nodes }
            }
            Command::SetFlows { flows } => Command::SetFlows { flows: std::mem::replace(&mut doc.flows, flows) },
            Command::SetStyles { styles, style_meta } => {
                Command::SetStyles { styles: std::mem::replace(&mut doc.styles, styles), style_meta: std::mem::replace(&mut doc.style_meta, style_meta) }
            }
            Command::SetComments { comments } => Command::SetComments { comments: std::mem::replace(&mut doc.comments, comments) },
            Command::SetLibraryState { state, library_page } => Command::SetLibraryState {
                state: std::mem::replace(&mut doc.library, state),
                library_page: std::mem::replace(&mut doc.library_page, library_page),
            },
            Command::SetGuides { page, guides } => {
                let old = if guides.is_empty() { doc.guides.remove(&page) } else { doc.guides.insert(page, guides) };
                Command::SetGuides { page, guides: old.unwrap_or_default() }
            }
            Command::SetPrototypeDevice { device } => Command::SetPrototypeDevice { device: std::mem::replace(&mut doc.prototype_device, device) },
            Command::SetBoardSession { session } => Command::SetBoardSession { session: std::mem::replace(&mut doc.board_session, session) },
            Command::SetVariables { variables, collections } => Command::SetVariables {
                variables: std::mem::replace(&mut doc.variables, variables),
                collections: std::mem::replace(&mut doc.variable_collections, collections),
            },
            Command::SetNodeMotion { node, motion } => {
                let n = doc.get_mut(node).ok_or(DocError::NotFound(node))?;
                let mut motion = *motion;
                motion.normalize();
                Command::SetNodeMotion { node, motion: Box::new(std::mem::replace(&mut n.motion, motion)) }
            }
            Command::SetTimeline { page, settings } => {
                if doc.get(page).is_none() {
                    return Err(DocError::NotFound(page));
                }
                let settings = settings.sanitized();
                let old = if settings.is_default() { doc.timelines.remove(&page) } else { doc.timelines.insert(page, settings) };
                Command::SetTimeline { page, settings: old.unwrap_or_default() }
            }
            Command::Batch(cmds) => {
                let mut inv = Vec::with_capacity(cmds.len());
                for c in cmds {
                    match c.apply(doc) {
                        Ok(i) => inv.push(i),
                        Err(e) => {
                            // All or nothing: take back what already went in, newest first.
                            for i in inv.into_iter().rev() {
                                let _ = i.apply(doc);
                            }
                            return Err(e);
                        }
                    }
                }
                inv.reverse();
                Command::Batch(inv)
            }
        })
    }
}

impl Document {
    pub(crate) fn bump_ids(&mut self) {
        if let Some((id, _)) = self.nodes.last_key_value() {
            self.next_id = self.next_id.max(id.0);
        }
    }
}

/// Linear undo stack.
#[derive(Default)]
pub struct History {
    undo: Vec<Command>,
    redo: Vec<Command>,
}

impl History {
    pub fn exec(&mut self, doc: &mut Document, cmd: Command) -> Result<(), DocError> {
        let inv = cmd.apply(doc)?;
        self.undo.push(inv);
        self.redo.clear();
        Ok(())
    }
    pub fn undo(&mut self, doc: &mut Document) -> Result<bool, DocError> {
        let Some(c) = self.undo.pop() else { return Ok(false) };
        self.redo.push(c.apply(doc)?);
        Ok(true)
    }
    pub fn redo(&mut self, doc: &mut Document) -> Result<bool, DocError> {
        let Some(c) = self.redo.pop() else { return Ok(false) };
        self.undo.push(c.apply(doc)?);
        Ok(true)
    }
    /// Calls `f` on every stored command (undo and redo stacks, Batch contents included).
    /// For rewriting stored inverses when the document changes underneath them.
    pub fn for_each_command_mut(&mut self, mut f: impl FnMut(&mut Command)) {
        fn rec(c: &mut Command, f: &mut dyn FnMut(&mut Command)) {
            if let Command::Batch(cmds) = c {
                for c in cmds {
                    rec(c, f);
                }
            } else {
                f(c);
            }
        }
        for c in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            rec(c, &mut f);
        }
    }
    /// Number of undo steps available.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_update_and_remove() {
        let mut d = Document::new("t");
        let page = d.add_page("P");
        let id = d.alloc_id();
        d.insert(page, Node::rectangle(id, Rect::new(0.0, 0.0, 10.0, 10.0))).unwrap();
        let mut h = History::default();

        let mut n = d.get(id).unwrap().clone();
        n.name = "Renamed".into();
        h.exec(&mut d, Command::Update { node: Box::new(n) }).unwrap();
        assert_eq!(d.get(id).unwrap().name, "Renamed");
        h.undo(&mut d).unwrap();
        assert_eq!(d.get(id).unwrap().name, "Rectangle");

        h.exec(&mut d, Command::Remove { id }).unwrap();
        assert!(d.get(id).is_none());
        h.undo(&mut d).unwrap();
        assert_eq!(d.get(page).unwrap().children, vec![id]);
        h.redo(&mut d).unwrap();
        assert!(d.get(id).is_none());
    }

    #[test]
    fn failed_batch_rolls_back_and_records_nothing() {
        let mut d = Document::new("t");
        let page = d.add_page("P");
        let id = d.alloc_id();
        d.insert(page, Node::rectangle(id, Rect::new(0.0, 0.0, 10.0, 10.0))).unwrap();
        let before = d.clone();
        let mut renamed = d.get(id).unwrap().clone();
        renamed.name = "Renamed".into();
        let mut h = History::default();
        let batch = Command::Batch(vec![Command::Update { node: Box::new(renamed) }, Command::Remove { id }, Command::Remove { id: NodeId(9999) }]);
        assert!(h.exec(&mut d, batch).is_err());
        assert_eq!(d.nodes, before.nodes);
        assert_eq!(d.get(page).unwrap().children, vec![id]);
        assert!(!h.can_undo() && !h.can_redo());
    }

    #[test]
    fn undo_redo_flows() {
        let mut d = Document::new("t");
        let page = d.add_page("P");
        let fid = d.alloc_id();
        let f = d.insert(page, Node::frame(fid, Rect::new(0.0, 0.0, 10.0, 10.0))).unwrap();
        let flow = Flow { name: "Flow 1".into(), page, start: f, description: String::new() };
        let mut h = History::default();
        h.exec(&mut d, Command::SetFlows { flows: vec![flow.clone()] }).unwrap();
        assert_eq!(d.flows, vec![flow]);
        h.undo(&mut d).unwrap();
        assert!(d.flows.is_empty());
        h.redo(&mut d).unwrap();
        assert_eq!(d.flows.len(), 1);
    }

    #[test]
    fn undo_redo_pages() {
        let mut d = Document::new("t");
        let p1 = d.add_page("P1");
        let pid = d.alloc_id();
        let mut page = Node::new(pid, NodeKind::Page { background: Color::WHITE });
        page.name = "P2".into();
        let mut h = History::default();
        h.exec(&mut d, Command::InsertPage { index: 0, nodes: vec![page] }).unwrap();
        assert_eq!(d.pages, vec![pid, p1]);
        let rid = d.alloc_id();
        d.insert(pid, Node::rectangle(rid, Rect::new(0.0, 0.0, 1.0, 1.0))).unwrap();

        h.exec(&mut d, Command::RemovePage { id: pid }).unwrap();
        assert_eq!(d.pages, vec![p1]);
        assert!(d.get(rid).is_none());
        h.undo(&mut d).unwrap();
        assert_eq!(d.pages, vec![pid, p1]);
        assert_eq!(d.get(pid).unwrap().children, vec![rid]);
        h.undo(&mut d).unwrap();
        assert_eq!(d.pages, vec![p1]);
    }
}

#[cfg(test)]
mod style_tests {
    use crate::*;

    #[test]
    fn set_styles_undo() {
        let mut d = Document::new("t");
        d.add_page("P");
        let mut h = History::default();
        let mut styles = d.styles.clone();
        styles.insert("s1".into(), Style::Fill(vec![Paint::solid(Color::WHITE)]));
        let mut meta = d.style_meta.clone();
        meta.insert("s1".into(), StyleMeta { name: "Brand/White".into(), ..Default::default() });
        h.exec(&mut d, Command::SetStyles { styles, style_meta: meta }).unwrap();
        assert_eq!(d.style_name("s1"), "Brand/White");
        h.undo(&mut d).unwrap();
        assert!(d.styles.is_empty() && d.style_meta.is_empty());
        h.redo(&mut d).unwrap();
        assert_eq!(d.styles.len(), 1);
    }
}
