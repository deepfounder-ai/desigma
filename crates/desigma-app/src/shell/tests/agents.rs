use super::*;
use crate::agents::Link;

/// A closed port: the worker answers "Down" at once without any server on the machine.
const DEAD: &str = "http://127.0.0.1:9";

/// Frames until the worker thread has answered (connection refused is immediate).
fn settle(s: &mut Sim) {
    for _ in 0..100 {
        s.frame(vec![]);
        if !matches!(s.app().agents.link, Link::Connecting) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn agents_rail_opens_the_panel_and_explains_a_missing_server() {
    let mut s = Sim::new();
    s.shell.app.agents.url = DEAD.into();
    let i = rail::ITEMS.iter().position(|(item, ..)| *item == crate::app::RailItem::Agents).unwrap();
    let y = rail::cell_tops()[i] + 24.0;
    s.click(Pos2::new(24.0, y));
    assert_eq!(s.app().ui.rail, Some(crate::app::RailItem::Agents));
    assert!(s.rect_of("agents-panel").width() > 200.0, "the panel takes the left column");
    settle(&mut s);
    assert!(matches!(s.app().agents.link, Link::Down(_)), "{:?}", s.app().agents.link);
    // the explanation, the start hint and a Connect button are on screen; no messages list
    assert!(s.rect_of("agents-start-hint").height() > 0.0);
    assert!(s.rect_of("agents-retry").width() > 0.0);
    assert!(s.ctx.data(|d| d.get_temp::<Rect>(egui::Id::new(("test-rect", "agents-messages".to_string())))).is_none());
    // clicking the rail item again closes the panel and the layers come back
    s.click(Pos2::new(24.0, y));
    assert_eq!(s.app().ui.rail, None);
}

#[test]
fn agents_panel_preset_and_notes_tab() {
    let s = Sim::with(Options { panel: Some("agents-notes".into()), ..Default::default() });
    assert_eq!(s.app().ui.rail, Some(crate::app::RailItem::Agents));
    assert_eq!(s.app().agents.tab, crate::agents::Tab::Notes);
    // the composer defaults are set once the worker started
    assert_eq!(s.app().agents.parallel_count, 1);
    assert!(s.app().agents.use_selection);
}
