//! 不屬於 Swift 移植的管理器測試（優先序同分、選取狀態、通知與並行契約）。
//! 獨立成檔，讓 parity.tsv 對 `modules.rs` 的計數只含移植案例。

use super::fakes::FakeModule;
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn counter(manager: &ModuleManager) -> (Arc<AtomicUsize>, Subscription) {
    let n = Arc::new(AtomicUsize::new(0));
    let n2 = n.clone();
    let sub = manager.subscribe(Box::new(move || {
        n2.fetch_add(1, Ordering::SeqCst);
    }));
    (n, sub)
}

#[test]
fn highest_priority_active_module_wins_and_ties_go_to_the_first_registered() {
    let manager = ModuleManager::new();
    manager.register(FakeModule::new("first", 5, true, 0.0));
    manager.register(FakeModule::new("second", 5, true, 0.0));
    assert_eq!(manager.active_module_id().as_deref(), Some("first"));

    manager.register(FakeModule::new("top", 9, true, 0.0));
    assert_eq!(manager.active_module_id().as_deref(), Some("top"));
    manager.register(FakeModule::new("inactive-higher", 99, false, 0.0));
    assert_eq!(manager.active_module_id().as_deref(), Some("top"));
}

#[test]
fn clamped_preview_extra_handles_non_finite_and_boundary_values() {
    assert_eq!(clamped_preview_extra(f64::NAN), 0.0);
    assert_eq!(clamped_preview_extra(f64::INFINITY), 56.0);
    assert_eq!(clamped_preview_extra(f64::NEG_INFINITY), 0.0);
    assert_eq!(clamped_preview_extra(99.0), 56.0);
    assert_eq!(clamped_preview_extra(10.0), 10.0);
}

#[test]
fn register_runs_install_then_start_once() {
    let manager = ModuleManager::new();
    let module = FakeModule::new("a", 0, false, 0.0);
    manager.register(module.clone());
    assert_eq!(module.installed.load(Ordering::SeqCst), 1);
    assert_eq!(module.started.load(Ordering::SeqCst), 1);
}

#[test]
fn registering_a_duplicate_id_is_ignored() {
    let manager = ModuleManager::new();
    let first = FakeModule::new("a", 0, true, 10.0);
    let dup = FakeModule::new("a", 50, true, 20.0);
    manager.register(first);
    manager.register(dup.clone());
    assert_eq!(manager.installed_module_ids(), ["a"]);
    assert_eq!(manager.preview_extra_height(), 10.0);
    assert_eq!(dup.started.load(Ordering::SeqCst), 0);
}

#[test]
fn installed_module_ids_keep_registration_order() {
    let manager = ModuleManager::new();
    manager.register(FakeModule::new("b", 1, false, 0.0));
    manager.register(FakeModule::new("a", 2, false, 0.0));
    assert_eq!(manager.installed_module_ids(), ["b", "a"]);
}

#[test]
fn subscribers_are_notified_only_when_active_or_height_changes() {
    let manager = ModuleManager::new();
    let (n, _sub) = counter(&manager);
    let module = FakeModule::new("a", 0, false, 24.0);

    manager.register(module.clone());
    assert_eq!(
        n.load(Ordering::SeqCst),
        0,
        "inactive 且高度仍是 0：沒有變化"
    );

    module.set_active(true);
    assert_eq!(n.load(Ordering::SeqCst), 1);
    module.set_active(true); // 模組重複通知同一個值
    assert_eq!(n.load(Ordering::SeqCst), 1);
    module.set_extra_height(10.0);
    assert_eq!(n.load(Ordering::SeqCst), 2);
    module.set_extra_height(99.0); // 夾到 56
    assert_eq!(n.load(Ordering::SeqCst), 3);
    module.set_extra_height(500.0); // 仍是 56：沒有變化
    assert_eq!(n.load(Ordering::SeqCst), 3);
}

#[test]
fn a_module_that_is_active_at_registration_notifies_once() {
    let manager = ModuleManager::new();
    let (n, _sub) = counter(&manager);
    manager.register(FakeModule::new("a", 0, true, 24.0));
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[test]
fn selection_starts_empty_and_select_picks_an_installed_module() {
    let manager = ModuleManager::new();
    manager.register(FakeModule::new("a", 1, false, 0.0));
    manager.register(FakeModule::new("b", 2, false, 0.0));
    assert_eq!(manager.selected_module_id(), None);

    manager.select("b");
    assert_eq!(manager.selected_module_id().as_deref(), Some("b"));
    manager.select("a");
    assert_eq!(manager.selected_module_id().as_deref(), Some("a"));
}

#[test]
fn select_ignores_unknown_ids() {
    let manager = ModuleManager::new();
    manager.register(FakeModule::new("a", 1, false, 0.0));
    manager.select("a");
    manager.select("nope");
    assert_eq!(manager.selected_module_id().as_deref(), Some("a"));
}

#[test]
fn reset_selection_to_active_follows_the_active_module() {
    let manager = ModuleManager::new();
    let low = FakeModule::new("low", 1, true, 0.0);
    let high = FakeModule::new("high", 2, false, 0.0);
    manager.register(low);
    manager.register(high.clone());
    manager.select("high");

    manager.reset_selection_to_active();
    assert_eq!(manager.selected_module_id().as_deref(), Some("low"));

    high.set_active(true);
    manager.select("low");
    manager.reset_selection_to_active();
    assert_eq!(manager.selected_module_id().as_deref(), Some("high"));
}

#[test]
fn reset_selection_without_active_module_clears_it() {
    let manager = ModuleManager::new();
    manager.register(FakeModule::new("a", 1, false, 0.0));
    manager.select("a");
    manager.reset_selection_to_active();
    assert_eq!(manager.selected_module_id(), None);
}

#[test]
fn selection_changes_notify_but_repeats_do_not() {
    let manager = ModuleManager::new();
    manager.register(FakeModule::new("a", 1, true, 0.0));
    manager.register(FakeModule::new("b", 0, false, 0.0));
    let (n, _sub) = counter(&manager);

    manager.select("b");
    assert_eq!(n.load(Ordering::SeqCst), 1);
    manager.select("b");
    assert_eq!(n.load(Ordering::SeqCst), 1);
    manager.select("nope");
    assert_eq!(n.load(Ordering::SeqCst), 1);
    manager.reset_selection_to_active();
    assert_eq!(n.load(Ordering::SeqCst), 2);
    manager.reset_selection_to_active();
    assert_eq!(n.load(Ordering::SeqCst), 2);
}

#[test]
fn dropping_a_subscription_stops_notifications() {
    let manager = ModuleManager::new();
    let module = FakeModule::new("a", 0, false, 0.0);
    manager.register(module.clone());
    let (n, sub) = counter(&manager);
    module.set_active(true);
    assert_eq!(n.load(Ordering::SeqCst), 1);
    drop(sub);
    module.set_active(false);
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[test]
fn dropping_the_manager_unsubscribes_from_modules() {
    let manager = ModuleManager::new();
    let module = FakeModule::new("a", 0, false, 0.0);
    manager.register(module.clone());
    let (n, sub) = counter(&manager);
    drop(manager);

    module.set_active(true); // 不得 panic，也不得再通知
    assert_eq!(n.load(Ordering::SeqCst), 0);
    drop(sub); // 訂閱比管理器活得久也要能乾淨 drop
}

#[test]
fn subscriber_may_read_and_reenter_the_manager_without_deadlock() {
    let manager = Arc::new(ModuleManager::new());
    let module = FakeModule::new("a", 0, false, 24.0);
    manager.register(module.clone());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (weak, s) = (Arc::downgrade(&manager), seen.clone());
    let _sub = manager.subscribe(Box::new(move || {
        let m = weak.upgrade().unwrap();
        // 在回呼內讀狀態、改選取、甚至註冊新模組：持鎖通知的話都會死結。
        lock(&s).push((m.active_module_id(), m.preview_extra_height()));
        m.select("a");
        if m.installed_module_ids().len() == 1 {
            m.register(FakeModule::new("late", -1, false, 0.0));
        }
    }));

    module.set_active(true);

    assert_eq!(lock(&seen)[0], (Some("a".to_string()), 24.0));
    assert_eq!(manager.installed_module_ids(), ["a", "late"]);
}

#[test]
fn concurrent_module_changes_end_in_a_consistent_state() {
    let manager = Arc::new(ModuleManager::new());
    let a = FakeModule::new("a", 1, false, 0.0);
    let b = FakeModule::new("b", 2, false, 0.0);
    manager.register(a.clone());
    manager.register(b.clone());

    let threads: Vec<_> = [(a.clone(), 10.0), (b.clone(), 20.0)]
        .into_iter()
        .map(|(m, h)| {
            std::thread::spawn(move || {
                for i in 0..500 {
                    m.set_active(i % 2 == 0);
                    m.set_extra_height(h);
                }
                m.set_active(true);
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }

    assert_eq!(manager.active_module_id().as_deref(), Some("b"));
    assert_eq!(manager.preview_extra_height(), 20.0);
}
