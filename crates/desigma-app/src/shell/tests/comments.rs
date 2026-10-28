//! UI tests for comments: the Comment tool, pins, threads, the panel, undo.

use super::*;
use crate::comments::model;
use crate::util::world_aabb;
use desigma_doc::CommentThread;

fn sim() -> Sim {
    let mut s = Sim::new();
    s.shell.app.comments.author = Some("Kir".into());
    s
}

/// Screen point inside the named top-level frame.
fn inside(s: &Sim, name: &str) -> Pos2 {
    let b = world_aabb(&s.app().doc, s.by_name(name));
    s.screen(b.x + 40.0, b.y + 40.0)
}

fn ctrl_enter(s: &mut Sim) {
    s.mods = Modifiers::COMMAND;
    s.key(Key::Enter);
    s.mods = Modifiers::NONE;
}

/// Comment tool, click a frame, type, post. Returns the new thread's id.
fn post(s: &mut Sim, frame: &str, text: &str) -> String {
    s.key(Key::C);
    assert_eq!(s.app().tool, Tool::Comment);
    let at = inside(s, frame);
    s.click(at);
    assert!(s.app().comments.composer.is_some(), "the click opened the composer");
    s.type_text(text);
    s.frame(vec![]);
    ctrl_enter(s);
    s.frame(vec![]);
    s.app().doc.comments.last().expect("a thread").id.clone()
}

/// Centre of a thread's bubble on screen.
fn bubble(s: &Sim, id: &str) -> Pos2 {
    let t = s.app().doc.comments.iter().find(|t| t.id == id).unwrap();
    let tip = s.app().view.to_screen(s.app().canvas_rect.min, model::pin_point(&s.app().doc, &t.anchor).unwrap());
    tip + egui::vec2(theme::COMMENT_PIN / 2.0, -theme::COMMENT_PIN / 2.0)
}

#[test]
fn posting_a_comment_anchors_it_to_the_node_under_the_cursor() {
    let mut s = sim();
    let at = inside(&s, "Welcome");
    let id = post(&mut s, "Welcome", "Make this bigger");
    let app = s.app();
    assert_eq!(app.doc.comments.len(), 1);
    let t = &app.doc.comments[0];
    assert_eq!(t.messages[0].text, "Make this bigger");
    assert_eq!(t.messages[0].author, "Kir");
    let hit = crate::canvas::interact::hit_under(app, app.view.to_page(app.canvas_rect.min, at), false);
    assert!(hit.is_some() && t.anchor.node == hit, "anchored to the layer under the cursor");
    let node_box = world_aabb(&app.doc, t.anchor.node.unwrap());
    let page_pt = app.view.to_page(app.canvas_rect.min, at);
    assert!(
        (t.anchor.offset.x - (page_pt.x - node_box.x)).abs() < 2.0 && (t.anchor.offset.y - (page_pt.y - node_box.y)).abs() < 2.0,
        "offset is relative to the layer: {:?}",
        t.anchor.offset
    );
    assert!(app.comments.composer.is_none(), "the composer closed");
    assert_eq!(app.tool, Tool::Comment, "comment mode stays on");
    assert!(!app.comments.pin_rects.is_empty(), "the pin is drawn");
    assert_eq!(id, t.id);
    // one undo step takes it back
    s.cmd_key(Key::Z);
    assert!(s.app().doc.comments.is_empty());
    s.cmd_key(Key::Z);
    s.mods = Modifiers::COMMAND | Modifiers::SHIFT;
    s.key(Key::Z);
    s.mods = Modifiers::NONE;
    assert_eq!(s.app().doc.comments.len(), 1, "redo brings it back");
}

#[test]
fn a_click_on_empty_canvas_pins_to_the_page() {
    let mut s = sim();
    s.key(Key::C);
    let at = s.app().canvas_rect.min + egui::vec2(12.0, 12.0);
    let page = s.app().view.to_page(s.app().canvas_rect.min, at);
    assert!(crate::canvas::interact::hit_under(s.app(), page, false).is_none(), "empty spot");
    s.click(at);
    s.type_text("Free note");
    s.frame(vec![]);
    ctrl_enter(&mut s);
    let t = &s.app().doc.comments[0];
    assert_eq!(t.anchor.node, None);
    assert!((t.anchor.offset.x - page.x).abs() < 2.0 && (t.anchor.offset.y - page.y).abs() < 2.0);
}

#[test]
fn escape_cancels_the_composer_and_empty_text_cannot_be_posted() {
    let mut s = sim();
    s.key(Key::C);
    let at = inside(&s, "Welcome");
    s.click(at);
    assert!(s.app().comments.composer.is_some());
    ctrl_enter(&mut s);
    assert!(s.app().doc.comments.is_empty(), "nothing to post");
    assert!(s.app().comments.composer.is_some(), "the composer stays open");
    s.type_text("draft");
    s.frame(vec![]);
    s.key(Key::Escape);
    s.frame(vec![]);
    assert!(s.app().comments.composer.is_none());
    assert!(s.app().doc.comments.is_empty());
    assert_eq!(s.app().tool, Tool::Comment, "Esc closed the draft, not the tool");
}

#[test]
fn clicking_a_pin_opens_the_thread_and_a_reply_is_added() {
    let mut s = sim();
    let id = post(&mut s, "Welcome", "First");
    s.shell.app.set_tool(Tool::Move);
    s.click(bubble(&s, &id));
    assert_eq!(s.app().comments.open.as_deref(), Some(id.as_str()));
    s.rect_of("comment-reply-box");
    s.type_text("Second message");
    s.frame(vec![]);
    ctrl_enter(&mut s);
    s.frame(vec![]);
    let t = &s.app().doc.comments[0];
    assert_eq!(t.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["First", "Second message"]);
    assert!(s.app().comments.reply.is_empty(), "the reply box was cleared");
    s.shell.app.undo();
    assert_eq!(s.app().doc.comments[0].messages.len(), 1, "the reply is one undo step");
}

#[test]
fn the_reply_button_posts_too() {
    let mut s = sim();
    let id = post(&mut s, "Welcome", "First");
    s.click(bubble(&s, &id));
    s.type_text("via button");
    s.frame(vec![]);
    let b = s.rect_of("comment-reply-post");
    s.click(b.center());
    assert_eq!(s.app().doc.comments[0].messages.len(), 2);
}

#[test]
fn resolving_hides_the_pin_unless_show_resolved_is_on() {
    let mut s = sim();
    let id = post(&mut s, "Welcome", "Fix me");
    s.click(bubble(&s, &id));
    let r = s.rect_of("comment-resolve");
    s.click(r.center());
    assert!(s.app().doc.comments[0].resolved);
    assert!(s.app().comments.open.is_none(), "the popover closed");
    assert!(s.app().comments.pin_rects.is_empty(), "resolved pins are hidden");
    s.shell.app.comments.show_resolved = true;
    s.frame(vec![]);
    assert_eq!(s.app().comments.pin_rects.len(), 1);
    s.cmd_key(Key::Z);
    assert!(!s.app().doc.comments[0].resolved, "undo reopens the thread");
}

#[test]
fn the_message_menu_edits_and_deletes() {
    let mut s = sim();
    let id = post(&mut s, "Welcome", "Original");
    s.shell.app.reply_comment(&id, "Reply text");
    s.click(bubble(&s, &id));
    // "…" of the first message, then Edit (first row)
    let dots = s.rect_of("comment-dots-0");
    s.click(dots.center());
    assert!(s.app().comments.menu.is_some());
    s.key(Key::ArrowDown);
    s.key(Key::Enter);
    assert!(s.app().comments.editing.is_some(), "Edit opened an editor");
    // the editor holds the old text; select all and replace it
    s.mods = Modifiers::COMMAND;
    s.key(Key::A);
    s.mods = Modifiers::NONE;
    s.type_text("Edited");
    s.frame(vec![]);
    ctrl_enter(&mut s);
    s.frame(vec![]);
    assert_eq!(s.app().doc.comments[0].messages[0].text, "Edited");
    assert!(s.app().doc.comments[0].messages[0].edited.is_some());

    // Delete message (second row) on the reply
    let dots = s.rect_of("comment-dots-1");
    s.click(dots.center());
    s.key(Key::ArrowDown);
    s.key(Key::ArrowDown);
    s.key(Key::Enter);
    assert_eq!(s.app().doc.comments[0].messages.len(), 1);

    // Delete thread (third row)
    let dots = s.rect_of("comment-dots-0");
    s.click(dots.center());
    for _ in 0..3 {
        s.key(Key::ArrowDown);
    }
    s.key(Key::Enter);
    assert!(s.app().doc.comments.is_empty());
    assert!(s.app().comments.open.is_none());
    // everything is undoable, one step at a time
    s.cmd_key(Key::Z);
    assert_eq!(s.app().doc.comments.len(), 1);
}

#[test]
fn pins_follow_their_node_when_it_moves() {
    let mut s = sim();
    let id = post(&mut s, "Welcome", "Stay with the frame");
    let before = bubble(&s, &id);
    let frame = s.by_name("Welcome");
    s.shell.app.edit_nodes(&[frame], |n| n.transform.tx += 100.0);
    s.frame(vec![]);
    let after = bubble(&s, &id);
    let dx = (after.x - before.x) / s.app().view.zoom;
    assert!((dx - 100.0).abs() < 0.5, "moved {dx}");
    let r = s.app().comments.pin_rects[0];
    assert!(r.contains(after - egui::vec2(0.0, 1.0)) || r.contains(after), "the drawn pin moved too");
}

#[test]
fn the_panel_lists_threads_and_clicking_one_goes_to_its_pin() {
    let mut s = sim();
    let a = post(&mut s, "Welcome", "Alpha note");
    s.key(Key::Escape);
    let page2 = s.app().doc.pages[1];
    let anchor = model::make_anchor(&s.app().doc, page2, desigma_doc::Vec2::new(2000.0, 1500.0), None);
    let b = s.shell.app.post_comment(anchor, "Beta on the second page").unwrap();
    s.shell.app.set_tool(Tool::Comment);
    s.frame(vec![]);
    s.rect_of("comments-panel");
    // "This page" lists only the first thread
    assert!(s.app().doc.comments.iter().any(|t| t.id == a) && s.app().doc.comments.iter().any(|t| t.id == b));
    s.rect_of("comment-card-Alpha note");
    let all = s.rect_of("comments-filter-pages");
    s.click(Pos2::new(all.right() - 20.0, all.center().y));
    assert!(s.app().comments.all_pages);
    let card = s.rect_of("comment-card-Beta on the second page");
    s.click(card.center());
    assert_eq!(s.app().page, page2, "switched to the thread's page");
    assert_eq!(s.app().comments.open.as_deref(), Some(b.as_str()));
    let c = bubble(&s, &b);
    assert!(s.app().canvas_rect.contains(c), "the pin is on screen: {c:?}");
}

#[test]
fn the_panel_filters_by_status_and_search() {
    let mut s = sim();
    let a = post(&mut s, "Welcome", "Alpha note");
    s.key(Key::Escape);
    s.key(Key::C);
    let at = inside(&s, "Welcome") + egui::vec2(120.0, 120.0);
    s.click(at);
    s.type_text("Beta note");
    s.frame(vec![]);
    ctrl_enter(&mut s);
    s.shell.app.resolve_comment_thread(&a, true);
    s.frame(vec![]);
    s.rect_of("comment-card-Beta note");
    let status = s.rect_of("comments-filter-status");
    s.click(Pos2::new(status.right() - 20.0, status.center().y));
    assert!(s.app().comments.list_resolved);
    s.rect_of("comment-card-Alpha note");
    // search narrows the list
    s.click(Pos2::new(status.left() + 20.0, status.center().y));
    let search = s.rect_of("comments-search");
    s.click(search.center());
    s.type_text("zzz");
    s.frame(vec![]);
    assert_eq!(s.app().comments.search, "zzz");
    let filter = model::Filter { all_pages: false, resolved: false, query: "zzz".into() };
    assert!(model::filtered(&s.app().doc, s.app().page, &s.app().doc.comments, &filter).is_empty());
}

#[test]
fn leaving_comment_mode_drops_a_draft_and_keeps_pins() {
    let mut s = sim();
    post(&mut s, "Welcome", "Kept");
    s.key(Key::C);
    let at = inside(&s, "Welcome") + egui::vec2(200.0, 200.0);
    s.click(at);
    assert!(s.app().comments.composer.is_some());
    s.shell.app.set_tool(Tool::Move);
    s.frame(vec![]);
    assert!(s.app().comments.composer.is_none());
    assert_eq!(s.app().doc.comments.len(), 1);
    assert_eq!(s.app().comments.pin_rects.len(), 1, "pins stay visible in other tools");
}

#[test]
fn comments_survive_a_save_and_load() {
    let mut s = sim();
    post(&mut s, "Welcome", "Persisted");
    let json = s.app().doc.to_json().unwrap();
    let back = desigma_doc::Document::from_json(&json).unwrap();
    let t: &CommentThread = &back.comments[0];
    assert_eq!(t.messages[0].text, "Persisted");
    assert!(t.anchor.node.is_some());
}

#[test]
fn the_toolbar_button_and_c_both_start_comment_mode() {
    let mut s = sim();
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1088.0));
    let gi = crate::canvas::tools::GROUPS.iter().position(|g| g.contains(&Tool::Comment)).unwrap();
    s.click(crate::panels::toolbar::group_button_rect(screen, gi).center());
    assert_eq!(s.app().tool, Tool::Comment);
    s.rect_of("comments-panel");
    s.key(Key::V);
    assert_eq!(s.app().tool, Tool::Move);
    s.key(Key::C);
    assert_eq!(s.app().tool, Tool::Comment);
}

#[test]
fn your_name_is_asked_in_preferences_and_used_for_new_comments() {
    let mut s = Sim::new();
    run_action(&mut s, crate::panels::menus::Action::YourName);
    assert!(s.app().dialog.is_some(), "a name prompt opened");
    s.frame(vec![]);
    s.type_text("Ada Lovelace");
    s.frame(vec![]);
    s.key(Key::Enter);
    s.frame(vec![]);
    assert!(s.app().dialog.is_none());
    assert_eq!(s.app().comments.author.as_deref(), Some("Ada Lovelace"));
    let id = post(&mut s, "Welcome", "Hello");
    let t = s.app().doc.comments.iter().find(|t| t.id == id).unwrap();
    assert_eq!(t.messages[0].author, "Ada Lovelace");
}

fn run_action(s: &mut Sim, a: crate::panels::menus::Action) {
    let ctx = s.ctx.clone();
    crate::panels::menus::run(&mut s.shell.app, &ctx, a);
    s.frame(vec![]);
}

#[test]
fn task_markers_cycle_from_the_popover_and_the_panel() {
    use model::TaskStatus::*;
    let mut s = sim();
    let id = post(&mut s, "Welcome", "[ ] Tighten the hero @ada");
    let status = |s: &Sim| model::task_status(&s.app().doc.comments[0]);
    assert_eq!(status(&s), Some(Todo));
    s.click(bubble(&s, &id));
    assert_eq!(s.app().comments.open.as_deref(), Some(id.as_str()));
    let st = s.rect_of("comment-task-status");
    s.click(st.center());
    assert_eq!(status(&s), Some(Doing), "the header control moves the task on");
    assert_eq!(s.app().comments.open.as_deref(), Some(id.as_str()), "the popover stays open");
    s.key(Key::Escape);
    s.frame(vec![]);
    let mark = s.rect_of("comment-mark-[/] Tighten the hero @ad");
    s.click(mark.center());
    assert_eq!(status(&s), Some(Done), "the panel's Mark done rewrites the marker");
    assert_eq!(s.app().doc.comments[0].messages[0].text, "[x] Tighten the hero @ada");
    assert!(s.app().comments.open.is_none(), "the button did not open the thread");
    s.rect_of("comment-mark-[x] Tighten the hero @ad");
    s.shell.app.undo();
    assert_eq!(status(&s), Some(Doing), "each status change is one undo step");
}
