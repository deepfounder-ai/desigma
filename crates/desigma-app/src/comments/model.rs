//! Pure comment logic: anchors that follow nodes, thread edits, list filtering, task
//! markers and `@mentions`. Every edit returns the complete new thread list, ready for
//! `Command::SetComments`.

use super::time;
use desigma_doc::{Comment, CommentAnchor, CommentThread, Document, NodeId, Vec2};

/// Anchors a pin dropped at page point `point`: relative to `node` when there is one
/// (so the pin follows it), otherwise at the page point.
pub fn make_anchor(doc: &Document, page: NodeId, point: Vec2, node: Option<NodeId>) -> CommentAnchor {
    if let Some(n) = node.filter(|n| doc.get(*n).is_some()) {
        if let Some(inv) = doc.world_transform(n).invert() {
            return CommentAnchor { page, node: Some(n), offset: inv.apply(point) };
        }
    }
    CommentAnchor { page, node: None, offset: point }
}

/// Page the thread's pin is on: the page of its node, else the anchor's page.
pub fn thread_page(doc: &Document, t: &CommentThread) -> NodeId {
    t.anchor.node.and_then(|n| doc.page_of(n)).unwrap_or(t.anchor.page)
}

/// Page-space position of the pin. `None` when the anchor node was deleted.
pub fn pin_point(doc: &Document, a: &CommentAnchor) -> Option<Vec2> {
    match a.node {
        Some(n) => {
            doc.get(n)?;
            Some(doc.world_transform(n).apply(a.offset))
        }
        None => Some(a.offset),
    }
}

pub fn comment(id: String, author: &str, text: &str) -> Comment {
    Comment { id, author: author.to_string(), text: text.trim().to_string(), created: time::now_rfc3339(), edited: None }
}

pub fn add_thread(threads: &[CommentThread], t: CommentThread) -> Vec<CommentThread> {
    let mut v = threads.to_vec();
    v.push(t);
    v
}

fn edit_thread(threads: &[CommentThread], id: &str, f: impl FnOnce(&mut CommentThread)) -> Option<Vec<CommentThread>> {
    let mut v = threads.to_vec();
    f(v.iter_mut().find(|t| t.id == id)?);
    Some(v)
}

pub fn reply(threads: &[CommentThread], thread: &str, c: Comment) -> Option<Vec<CommentThread>> {
    edit_thread(threads, thread, |t| t.messages.push(c))
}

pub fn edit_comment(threads: &[CommentThread], thread: &str, comment: &str, text: &str) -> Option<Vec<CommentThread>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut hit = false;
    let out = edit_thread(threads, thread, |t| {
        if let Some(m) = t.messages.iter_mut().find(|m| m.id == comment) {
            if m.text != text {
                m.text = text.to_string();
                m.edited = Some(time::now_rfc3339());
            }
            hit = true;
        }
    })?;
    hit.then_some(out)
}

/// Removes one message; removing the only message removes the thread.
pub fn delete_comment(threads: &[CommentThread], thread: &str, comment: &str) -> Option<Vec<CommentThread>> {
    let t = threads.iter().find(|t| t.id == thread)?;
    if !t.messages.iter().any(|m| m.id == comment) {
        return None;
    }
    if t.messages.len() == 1 {
        return delete_thread(threads, thread);
    }
    edit_thread(threads, thread, |t| t.messages.retain(|m| m.id != comment))
}

pub fn delete_thread(threads: &[CommentThread], thread: &str) -> Option<Vec<CommentThread>> {
    threads.iter().any(|t| t.id == thread).then(|| threads.iter().filter(|t| t.id != thread).cloned().collect())
}

pub fn set_resolved(threads: &[CommentThread], thread: &str, resolved: bool) -> Option<Vec<CommentThread>> {
    let cur = threads.iter().find(|t| t.id == thread)?;
    if cur.resolved == resolved {
        return None;
    }
    edit_thread(threads, thread, |t| t.resolved = resolved)
}

/// Task state of a thread, read from a marker at the start of its opening message:
/// `[ ]` to do, `[/]` in progress, `[x]` done. A plain-text convention, so files and
/// collaboration sync are untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    Todo,
    Doing,
    Done,
}

impl TaskStatus {
    pub fn marker(self) -> &'static str {
        match self {
            TaskStatus::Todo => "[ ]",
            TaskStatus::Doing => "[/]",
            TaskStatus::Done => "[x]",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::Todo => "To do",
            TaskStatus::Doing => "In progress",
            TaskStatus::Done => "Done",
        }
    }

    /// To do → in progress → done → to do.
    pub fn next(self) -> TaskStatus {
        match self {
            TaskStatus::Todo => TaskStatus::Doing,
            TaskStatus::Doing => TaskStatus::Done,
            TaskStatus::Done => TaskStatus::Todo,
        }
    }
}

/// Splits a leading task marker off `text`: `"[x] Fix it"` is `(Some(Done), "Fix it")`.
pub fn split_marker(text: &str) -> (Option<TaskStatus>, &str) {
    let t = text.trim_start();
    let status = match t.as_bytes() {
        [b'[', b' ', b']', ..] => TaskStatus::Todo,
        [b'[', b'/', b']', ..] => TaskStatus::Doing,
        [b'[', b'x' | b'X', b']', ..] => TaskStatus::Done,
        _ => return (None, text),
    };
    let rest = &t[3..];
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return (None, text);
    }
    (Some(status), rest.trim_start())
}

/// The thread's task state, if its opening message starts with a marker.
pub fn task_status(t: &CommentThread) -> Option<TaskStatus> {
    t.messages.first().and_then(|m| split_marker(&m.text).0)
}

/// `text` with its task marker replaced (or added) for `status`.
pub fn with_status(text: &str, status: TaskStatus) -> String {
    let body = split_marker(text).1.trim();
    if body.is_empty() {
        status.marker().to_string()
    } else {
        format!("{} {body}", status.marker())
    }
}

fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Byte ranges of the `@name` tokens in `text` (letters, digits, `-` and `_`).
/// An `@` inside a word, as in an email address, is not a mention.
pub fn mention_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for (i, _) in text.match_indices('@') {
        if i > 0 && is_name_char(bytes[i - 1]) {
            continue;
        }
        let end = i + 1 + bytes[i + 1..].iter().take_while(|b| is_name_char(**b)).count();
        if end > i + 1 {
            out.push((i, end));
        }
    }
    out
}

/// Names mentioned in `text`, lower-cased, without the `@`, in order of appearance.
pub fn mentions(text: &str) -> Vec<String> {
    mention_spans(text).into_iter().map(|(a, b)| text[a + 1..b].to_lowercase()).collect()
}

/// Epoch milliseconds of the newest message (0 when unparsable).
pub fn last_activity(t: &CommentThread) -> u64 {
    t.messages.iter().filter_map(|m| time::parse_rfc3339(&m.created)).max().unwrap_or(0)
}

/// What the comments panel lists.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub all_pages: bool,
    /// List resolved threads instead of open ones.
    pub resolved: bool,
    pub query: String,
}

/// Threads matching `f`, newest activity first. A query of `@name` lists the threads
/// that mention exactly that name; any other query searches text and authors.
pub fn filtered<'a>(doc: &Document, page: NodeId, threads: &'a [CommentThread], f: &Filter) -> Vec<&'a CommentThread> {
    let q = f.query.trim().to_lowercase();
    let name = q.strip_prefix('@').filter(|n| !n.is_empty());
    let hit = |m: &Comment| match name {
        Some(n) => mentions(&m.text).iter().any(|x| x == n),
        None => m.text.to_lowercase().contains(&q) || m.author.to_lowercase().contains(&q),
    };
    let mut out: Vec<&CommentThread> = threads
        .iter()
        .filter(|t| t.resolved == f.resolved)
        .filter(|t| f.all_pages || thread_page(doc, t) == page)
        .filter(|t| q.is_empty() || t.messages.iter().any(hit))
        .collect();
    out.sort_by_key(|t| std::cmp::Reverse(last_activity(t)));
    out
}

/// Threads whose pin is drawn on `page`.
pub fn visible_on<'a>(doc: &'a Document, page: NodeId, threads: &'a [CommentThread], show_resolved: bool) -> impl Iterator<Item = &'a CommentThread> {
    threads.iter().filter(move |t| (show_resolved || !t.resolved) && thread_page(doc, t) == page && pin_point(doc, &t.anchor).is_some())
}

/// First letter of a name, upper-cased, for avatars.
pub fn initial(name: &str) -> String {
    name.trim().chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into())
}

/// Stable index into the avatar palette.
pub fn avatar_index(name: &str, palette: usize) -> usize {
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    h as usize % palette.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use desigma_doc::{Node, Rect};

    fn doc() -> (Document, NodeId, NodeId) {
        let mut d = Document::new("t");
        let page = d.add_page("P");
        let id = d.alloc_id();
        let n = d.insert(page, Node::frame(id, Rect::new(100.0, 50.0, 200.0, 100.0))).unwrap();
        (d, page, n)
    }

    fn thread(id: &str, anchor: CommentAnchor, texts: &[&str]) -> CommentThread {
        CommentThread {
            id: id.into(),
            anchor,
            messages: texts
                .iter()
                .enumerate()
                .map(|(i, t)| Comment {
                    id: format!("{id}-{i}"),
                    author: "Kir".into(),
                    text: t.to_string(),
                    created: time::format_rfc3339(1_000_000 + i as u64),
                    edited: None,
                })
                .collect(),
            resolved: false,
        }
    }

    #[test]
    fn node_anchored_pins_follow_the_node() {
        let (mut d, page, n) = doc();
        let a = make_anchor(&d, page, Vec2::new(130.0, 70.0), Some(n));
        assert_eq!(a.node, Some(n));
        assert_eq!((a.offset.x, a.offset.y), (30.0, 20.0), "offset is relative to the node's top-left");
        let p = pin_point(&d, &a).unwrap();
        assert_eq!((p.x, p.y), (130.0, 70.0));
        d.get_mut(n).unwrap().transform.tx += 40.0;
        d.get_mut(n).unwrap().transform.ty -= 10.0;
        let p = pin_point(&d, &a).unwrap();
        assert_eq!((p.x, p.y), (170.0, 60.0), "the pin moved with the frame");
    }

    #[test]
    fn page_anchored_pins_stay_and_deleted_nodes_hide_the_pin() {
        let (mut d, page, n) = doc();
        let free = make_anchor(&d, page, Vec2::new(5.0, 6.0), None);
        assert_eq!(pin_point(&d, &free).map(|p| (p.x, p.y)), Some((5.0, 6.0)));
        let a = make_anchor(&d, page, Vec2::new(130.0, 70.0), Some(n));
        d.remove(n).unwrap();
        assert!(pin_point(&d, &a).is_none());
        // an anchor to an unknown node falls back to a page point
        assert_eq!(make_anchor(&d, page, Vec2::new(1.0, 2.0), Some(n)).node, None);
    }

    #[test]
    fn rotated_parents_are_handled() {
        let (mut d, page, n) = doc();
        d.get_mut(n).unwrap().transform = desigma_doc::Affine::rotate(std::f32::consts::FRAC_PI_2);
        let a = make_anchor(&d, page, Vec2::new(10.0, 20.0), Some(n));
        let p = pin_point(&d, &a).unwrap();
        assert!((p.x - 10.0).abs() < 1e-3 && (p.y - 20.0).abs() < 1e-3);
        d.get_mut(n).unwrap().transform.tx = 40.0;
        let p = pin_point(&d, &a).unwrap();
        assert!((p.x - 50.0).abs() < 1e-3 && (p.y - 20.0).abs() < 1e-3);
    }

    #[test]
    fn reply_edit_resolve_delete() {
        let (d, page, _) = doc();
        let anchor = make_anchor(&d, page, Vec2::ZERO, None);
        let base = vec![thread("t1", anchor, &["first"]), thread("t2", anchor, &["other"])];
        let r = reply(&base, "t1", comment("c9".into(), "Ann", "  second ")).unwrap();
        assert_eq!(r[0].messages.len(), 2);
        assert_eq!(r[0].messages[1].text, "second", "text is trimmed");
        assert_eq!(base[0].messages.len(), 1, "inputs are not mutated");
        let e = edit_comment(&r, "t1", "c9", "second, edited").unwrap();
        assert_eq!(e[0].messages[1].text, "second, edited");
        assert!(e[0].messages[1].edited.is_some());
        assert!(edit_comment(&r, "t1", "c9", "   ").is_none(), "empty edits are rejected");
        assert!(edit_comment(&r, "t1", "nope", "x").is_none());
        let res = set_resolved(&e, "t1", true).unwrap();
        assert!(res[0].resolved && !res[1].resolved);
        assert!(set_resolved(&res, "t1", true).is_none(), "no-op is not an edit");
        let d1 = delete_comment(&e, "t1", "c9").unwrap();
        assert_eq!(d1[0].messages.len(), 1);
        let d2 = delete_comment(&d1, "t1", "t1-0").unwrap();
        assert_eq!(d2.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["t2"], "deleting the only message removes the thread");
        assert_eq!(delete_thread(&e, "t2").unwrap().len(), 1);
        assert!(delete_thread(&e, "zzz").is_none());
    }

    #[test]
    fn filters_pages_status_and_search() {
        let mut d = Document::new("t");
        let p1 = d.add_page("One");
        let p2 = d.add_page("Two");
        let a1 = make_anchor(&d, p1, Vec2::ZERO, None);
        let a2 = make_anchor(&d, p2, Vec2::ZERO, None);
        let mut t3 = thread("c", a1, &["Fix the Header spacing"]);
        t3.resolved = true;
        let ts = vec![thread("a", a1, &["old"]), thread("b", a2, &["on page two"]), t3, thread("d", a2, &["ping @Ada and @adam-k"])];
        let f = |all, resolved, q: &str| Filter { all_pages: all, resolved, query: q.into() };
        let ids = |v: Vec<&CommentThread>| v.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(filtered(&d, p1, &ts, &f(false, false, ""))), ["a"]);
        assert_eq!(ids(filtered(&d, p1, &ts, &f(true, false, ""))).len(), 3);
        assert_eq!(ids(filtered(&d, p1, &ts, &f(false, true, ""))), ["c"]);
        assert_eq!(ids(filtered(&d, p1, &ts, &f(true, false, "TWO"))), ["b"]);
        assert_eq!(ids(filtered(&d, p1, &ts, &f(true, true, "header"))), ["c"]);
        assert!(filtered(&d, p1, &ts, &f(true, false, "kir")).len() == 3, "authors are searched too");
        assert_eq!(ids(filtered(&d, p1, &ts, &f(true, false, "@ada"))), ["d"], "@name matches a mention of exactly that name");
        assert!(filtered(&d, p1, &ts, &f(true, false, "@adam")).is_empty());
        assert!(filtered(&d, p1, &ts, &f(true, false, "zzz")).is_empty());
        let shown: Vec<_> = visible_on(&d, p1, &ts, false).map(|t| t.id.as_str()).collect();
        assert_eq!(shown, ["a"]);
        let shown: Vec<_> = visible_on(&d, p1, &ts, true).map(|t| t.id.as_str()).collect();
        assert_eq!(shown, ["a", "c"]);
    }

    #[test]
    fn task_markers_on_the_opening_message() {
        use TaskStatus::*;
        assert_eq!(split_marker("[ ] Fix the gap"), (Some(Todo), "Fix the gap"));
        assert_eq!(split_marker("  [/]   half way"), (Some(Doing), "half way"));
        assert_eq!(split_marker("[x] shipped"), (Some(Done), "shipped"));
        assert_eq!(split_marker("[X]"), (Some(Done), ""));
        assert_eq!(split_marker("[x]not a marker"), (None, "[x]not a marker"));
        assert_eq!(split_marker("[?] no"), (None, "[?] no"));
        assert_eq!(split_marker("plain"), (None, "plain"));
        assert_eq!(split_marker(""), (None, ""));
        assert_eq!(with_status("[ ] Fix the gap", Done), "[x] Fix the gap");
        assert_eq!(with_status("no marker yet", Doing), "[/] no marker yet");
        assert_eq!(with_status("[x]  ", Todo), "[ ]");
        assert_eq!(Todo.next(), Doing);
        assert_eq!(Doing.next(), Done);
        assert_eq!(Done.next(), Todo);

        let (d, page, _) = doc();
        let anchor = make_anchor(&d, page, Vec2::ZERO, None);
        let t = thread("t", anchor, &["[/] doing", "[x] a reply marker does not count"]);
        assert_eq!(task_status(&t), Some(Doing));
        assert_eq!(task_status(&thread("u", anchor, &["plain"])), None);
        // the status is rewritten through the ordinary message edit, so it is one undo step
        let first = t.messages[0].id.clone();
        let done = edit_comment(std::slice::from_ref(&t), "t", &first, &with_status(&t.messages[0].text, Done)).unwrap();
        assert_eq!(task_status(&done[0]), Some(Done));
        assert_eq!(done[0].messages[0].text, "[x] doing");
    }

    #[test]
    fn mentions_are_name_tokens_after_an_at() {
        assert_eq!(mentions("@Ada can you check with @marcus-k and @m_2?"), ["ada", "marcus-k", "m_2"]);
        assert_eq!(mention_spans("hi @Ada!"), [(3, 7)]);
        assert!(mentions("mail me at kir@example.com").is_empty(), "an email is not a mention");
        assert!(mentions("@ alone and @").is_empty());
        assert_eq!(mentions("@ada @ada"), ["ada", "ada"]);
    }

    #[test]
    fn avatars() {
        assert_eq!(initial("  kir"), "K");
        assert_eq!(initial(""), "?");
        assert_eq!(avatar_index("Kir", 6), avatar_index("Kir", 6));
        assert!(avatar_index("Anything", 6) < 6);
    }
}
