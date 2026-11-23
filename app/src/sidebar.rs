//! The sidebar's own order and the tab Tiffin opens on, kept in the library's `Settings`. Folder sync keeps settings
//! local (storage.rs), so today they stay on this computer. Whatever is stored (by this build, a newer one or a hand
//! edit) is read through `order` and `launch_page`, which only ever answer with tabs this build has.
//! shortcut: a newer build's tab id is dropped when this build saves the order (only the preference is lost; the newer
//! build puts its tab back at its default place); keep unknown ids if that ever matters.

use crate::model::Settings;

/// Every sidebar tab in its default order. Settings is not a tab: it stays at the foot.
pub const TABS: [&str; 12] = ["home", "reports", "diary", "targets", "body", "recipes", "groceries", "pantry", "foods", "meals", "menus", "browser"];
/// The tab Tiffin opens on until the person picks another (owner, 2026-11-21).
pub const DEFAULT_LAUNCH: &str = "home";

pub const NOT_A_TAB: &str = "That isn't a sidebar tab.";

fn tab(id: &str) -> Option<&'static str> {
    TABS.iter().copied().find(|t| *t == id)
}

/// The order to draw: the stored ids this build knows, each once, with any tab missing from it (a new one, say) put back
/// after the tab it follows by default, so a new tab appears where it belongs rather than at the bottom.
pub fn order(s: &Settings) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = vec![];
    for id in s.sidebar_order.iter().filter_map(|id| tab(id)) {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    for (i, &t) in TABS.iter().enumerate() {
        if out.contains(&t) {
            continue;
        }
        let at = TABS[..i].iter().rev().find_map(|prev| out.iter().position(|o| o == prev)).map_or(0, |p| p + 1);
        out.insert(at, t);
    }
    out
}

/// The tab to open on: the stored one when this build has it, otherwise Home.
pub fn launch_page(s: &Settings) -> &'static str {
    tab(&s.launch_page).unwrap_or(DEFAULT_LAUNCH)
}

/// Store a new order. Unknown ids are refused (nothing is stored); missing tabs are filled in as [`order`] does. The
/// default order is stored as empty, so a later build's default (a new tab's place) still applies to it.
pub fn set_order(s: &mut Settings, ids: &[String]) -> Result<(), String> {
    if ids.iter().any(|id| tab(id).is_none()) {
        return Err(NOT_A_TAB.into());
    }
    let mut next = s.clone();
    next.sidebar_order = ids.to_vec();
    let clean = order(&next);
    s.sidebar_order = if clean == TABS { vec![] } else { clean.iter().map(|t| t.to_string()).collect() };
    Ok(())
}

/// Make `id` the tab Tiffin opens on. Home is stored as empty, the default.
pub fn set_launch_page(s: &mut Settings, id: &str) -> Result<(), String> {
    let t = tab(id).ok_or(NOT_A_TAB)?;
    s.launch_page = if t == DEFAULT_LAUNCH { String::new() } else { t.to_string() };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(order_ids: &[&str], launch: &str) -> Settings {
        Settings { sidebar_order: order_ids.iter().map(|s| s.to_string()).collect(), launch_page: launch.into(), ..Default::default() }
    }

    #[test]
    fn a_new_library_has_the_default_order_and_opens_on_home() {
        let s = Settings::default();
        assert_eq!(order(&s), TABS);
        assert_eq!(launch_page(&s), "home");
    }

    #[test]
    fn stored_orders_are_cleaned_and_new_tabs_go_where_they_belong() {
        // unknown and repeated ids drop out; every tab still appears exactly once
        let s = with(&["recipes", "nope", "recipes", "home", "settings"], "");
        let o = order(&s);
        assert_eq!(o[0], "recipes");
        assert!(o.iter().position(|t| *t == "recipes") < o.iter().position(|t| *t == "home"), "stored ids keep their order");
        assert_eq!(o.len(), TABS.len());
        assert!(TABS.iter().all(|t| o.contains(t)));
        // tabs missing from an older stored order (here "reports" and "diary") come back right after the tab each follows by default
        let older = with(&["browser", "home", "targets", "body", "recipes", "groceries", "pantry", "foods", "meals", "menus"], "");
        assert_eq!(&order(&older)[..4], ["browser", "home", "reports", "diary"]);
        // missing the first tab: it goes to the top
        let no_home = with(&["diary", "targets"], "");
        assert_eq!(order(&no_home)[0], "home");
    }

    #[test]
    fn setting_the_order_validates_and_stores_the_default_as_empty() {
        let mut s = Settings::default();
        let mut moved: Vec<String> = TABS.iter().map(|t| t.to_string()).collect();
        moved.swap(0, 4); // Recipes to the top, Home to Recipes' place
        assert_eq!(set_order(&mut s, &moved), Ok(()));
        assert_eq!(order(&s), moved.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(s.sidebar_order.len(), TABS.len(), "stored complete");
        let before = s.clone().sidebar_order;
        assert_eq!(set_order(&mut s, &["home".into(), "c:/windows".into()]), Err(NOT_A_TAB.into()));
        assert_eq!(s.sidebar_order, before, "nothing stored on a refusal");
        assert_eq!(set_order(&mut s, &TABS.map(String::from)), Ok(()));
        assert!(s.sidebar_order.is_empty(), "the default order is stored as empty");
    }

    #[test]
    fn the_launch_page_must_be_a_tab() {
        let mut s = Settings::default();
        assert_eq!(set_launch_page(&mut s, "diary"), Ok(()));
        assert_eq!((launch_page(&s), s.launch_page.as_str()), ("diary", "diary"));
        assert_eq!(set_launch_page(&mut s, "settings"), Err(NOT_A_TAB.into()));
        assert_eq!(launch_page(&s), "diary");
        assert_eq!(set_launch_page(&mut s, "home"), Ok(()));
        assert_eq!(s.launch_page, "", "Home is the default, stored as empty");
        // a stored page this build doesn't know (a newer build's tab, a hand edit) opens Home
        assert_eq!(launch_page(&with(&[], "reports-v9")), "home");
    }
}
