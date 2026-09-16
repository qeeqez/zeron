//! Headless tests for multi-project support: the Open Project action's
//! folder picker, per-project windows and chat isolation, the recent-projects
//! affordances (empty state + sidebar switcher), and the File-menu entry.
//! `HOME` is sandboxed per test so recents/chats never touch the real profile.

use std::any::TypeId;
use std::path::PathBuf;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, OwnedMenuItem, TestAppContext, VisualTestContext};

use crate::workspace::Workspace;

/// Redirect `~` into a throwaway dir; nextest runs each test in its own
/// process, so no other thread can observe HOME mid-write.
fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
}

/// A fresh project folder under the system temp dir (canonicalized, like
/// `Project::open` leaves it).
fn temp_project(leaf: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-proj-{}", std::process::id())).join(leaf);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// Mount a `Workspace` bound to `project` in a headless window — the test
/// twin of `lifecycle::open_workspace_window_for`.
fn mount_project<'a>(
    cx: &'a mut TestAppContext, project: &crate::project::Project,
) -> (Entity<Workspace>, AnyWindowHandle, &'a mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::for_project(project.clone(), window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    let handle = cx.update(|window, _| window.window_handle());
    (ws.unwrap(), handle, cx)
}

/// The `Workspace` entity of every open window, in `cx.windows()` order.
fn workspaces(cx: &TestAppContext) -> Vec<Entity<Workspace>> {
    cx.read(|cx| {
        cx.windows()
            .into_iter()
            .filter_map(|handle| {
                handle
                    .read::<Root, _, _>(cx, |root, cx| root.read(cx).view().clone().downcast::<Workspace>().ok())
                    .ok()
                    .flatten()
            })
            .collect()
    })
}

/// The project root of every open workspace window.
fn project_roots(cx: &TestAppContext) -> Vec<PathBuf> {
    workspaces(cx).iter().map(|ws| ws.read_with(cx, |ws, _| ws.project.root().to_path_buf())).collect()
}

/// The chat titles of every open workspace window.
fn chat_titles(cx: &TestAppContext) -> Vec<Vec<String>> {
    workspaces(cx)
        .iter()
        .map(|ws| ws.read_with(cx, |ws, _| ws.chats.iter().map(|c| c.title.to_string()).collect()))
        .collect()
}

/// The action type registered on a named menu item, if it is an action item.
fn menu_action(menu: &gpui_kit::OwnedMenu, name: &str) -> Option<TypeId> {
    menu.items.iter().find_map(|item| match item {
        OwnedMenuItem::Action { name: n, action, .. } if n == name => Some(action.as_any().type_id()),
        _ => None,
    })
}

#[test]
fn open_project_action_prompts_then_opens_a_window_on_that_folder() {
    let mut app = TestAppContext::single();
    sandbox_home();
    app.update(crate::install_app_actions);
    let a = temp_project("alpha");
    let b = temp_project("beta");
    let (_ws, handle, cx) = mount_project(&mut app, &crate::project::Project::open(&a));

    TestAppContext::dispatch_action(cx, handle, crate::OpenProject);
    assert!(cx.did_prompt_for_paths(), "Open Project should show the folder picker");

    // Cancelling leaves the window set unchanged.
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);

    TestAppContext::dispatch_action(cx, handle, crate::OpenProject);
    cx.simulate_path_prompt_response(|opts| {
        assert!(opts.directories && !opts.files, "the picker should select folders only");
        Some(vec![b.clone()])
    });
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 2, "picking a folder should open a second window");
    assert!(project_roots(cx).contains(&b), "the new window should be bound to the picked folder");
    // The picked folder is now the most recent project.
    assert_eq!(crate::recent_projects::list().first(), Some(&b));
}

#[test]
fn opening_an_already_open_project_focuses_instead_of_duplicating() {
    let mut app = TestAppContext::single();
    sandbox_home();
    let a = temp_project("alpha");
    let b = temp_project("beta");
    let (_ws_a, _h_a, cx) = mount_project(&mut app, &crate::project::Project::open(&a));
    let (_ws_b, _h_b, cx) = mount_project(cx, &crate::project::Project::open(&b));
    assert_eq!(cx.windows().len(), 2);

    cx.update(|_, cx| crate::lifecycle::open_project(&b, cx));
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 2, "re-opening a bound project must not open a second window");
    // …but it still counts as the most recently used project.
    assert_eq!(crate::recent_projects::list().first(), Some(&b));
}

#[test]
fn each_project_window_loads_its_own_chats() {
    let mut app = TestAppContext::single();
    sandbox_home();
    let a = temp_project("alpha");
    let b = temp_project("beta");
    // Seed one chat per project store before either window opens.
    let pa = crate::project::Project::open(&a);
    crate::persist::save_chats(&pa.chats_dir(), &[crate::model::Chat::new(0, "alpha chat")]);
    let pb = crate::project::Project::open(&b);
    crate::persist::save_chats(&pb.chats_dir(), &[crate::model::Chat::new(0, "beta chat")]);

    let (_ws_a, _h, cx) = mount_project(&mut app, &pa);
    let (_ws_b, _h, cx) = mount_project(cx, &pb);
    let mut titles = chat_titles(cx);
    titles.sort();
    assert_eq!(titles, vec![vec!["alpha chat".to_string()], vec!["beta chat".to_string()]]);
}

#[test]
fn empty_state_offers_open_project_and_recents() {
    let mut app = TestAppContext::single();
    sandbox_home();
    let a = temp_project("alpha");
    let b = temp_project("beta");
    crate::recent_projects::record(&a);
    crate::recent_projects::record(&b);
    let (_ws, _handle, cx) = mount_project(&mut app, &crate::project::Project::open(&b));

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("open-project").visible(), "empty state should offer Open Project");
        assert!(window.find("recent-projects").visible(), "empty state should list recent projects");
        // The current project is filtered out of its own recents list.
        assert!(window.try_find(format!("empty-recent-{}", a.display())).is_some());
        assert!(window.try_find(format!("empty-recent-{}", b.display())).is_none());
    });
}

#[test]
fn picking_a_recent_from_the_empty_state_opens_it() {
    let mut app = TestAppContext::single();
    sandbox_home();
    let a = temp_project("alpha");
    let b = temp_project("beta");
    crate::recent_projects::record(&a);
    let (_ws, _handle, cx) = mount_project(&mut app, &crate::project::Project::open(&b));

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click(format!("empty-recent-{}", a.display()), cx);
    });
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 2, "picking a recent should open its project window");
    assert!(project_roots(cx).contains(&a));
}

#[test]
fn sidebar_project_row_opens_the_switcher() {
    let mut app = TestAppContext::single();
    sandbox_home();
    let a = temp_project("alpha");
    let b = temp_project("beta");
    crate::recent_projects::record(&a);
    let (_ws, _handle, cx) = mount_project(&mut app, &crate::project::Project::open(&b));

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("project-switcher-btn", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("project-switcher").visible(), "the project row should open the switcher");
        assert!(window.find("project-open-folder").visible());
        assert!(window.find(format!("project-recent-{}", a.display())).visible());
    });
}

#[test]
fn switcher_open_folder_runs_the_picker() {
    let mut app = TestAppContext::single();
    sandbox_home();
    let a = temp_project("alpha");
    let (_ws, _handle, cx) = mount_project(&mut app, &crate::project::Project::open(&a));

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("project-switcher-btn", cx);
    });
    // The dialog's 250ms entrance animation leaves the row's observed bounds
    // stale — a click mid-animation lands on the overlay_closable backdrop and
    // dismisses the switcher instead of hitting the row. Settle past it.
    cx.executor().advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("project-open-folder", cx);
    });
    assert!(cx.did_prompt_for_paths(), "Open Folder… should run the native picker");
    let switcher_gone = cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.try_find("project-switcher").is_none()
    });
    assert!(switcher_gone, "the switcher should close when the picker opens");
}

#[test]
fn file_menu_offers_open_project() {
    let app = TestAppContext::single();
    app.update(|cx| cx.set_menus(crate::menus::app_menus()));
    let menus = app.read(|cx| cx.get_menus().expect("menus should be installed"));
    let file = menus.iter().find(|m| m.name == "File").expect("File menu");
    assert_eq!(menu_action(file, "Open Project…"), Some(TypeId::of::<crate::OpenProject>()));
}
