//! Opt-in synthetic timings. No native WebView, network or real vault contents.

use crate::{
    desktop::{
        context::edit_unlocked,
        view_test_support::{mounted, update},
    },
    records::{entry::Entry, folder::Folder, tree},
};
use dioxus::prelude::*;
use std::time::Instant;

fn sample(
    dom: &mut VirtualDom,
    action: impl FnOnce(crate::desktop::context::Session),
) -> (f64, f64, usize) {
    let start = Instant::now();
    dom.in_scope(ScopeId::APP, || action(consume_context()));
    let command = start.elapsed();
    let start = Instant::now();
    let mutations = dom.render_immediate_to_vec();
    (
        command.as_secs_f64() * 1000.0,
        start.elapsed().as_secs_f64() * 1000.0,
        mutations.edits.len(),
    )
}

fn report(size: usize, operation: &str, mut samples: Vec<(f64, f64, usize)>) {
    samples.sort_by(|a, b| a.1.total_cmp(&b.1));
    let median = samples[samples.len() / 2];
    let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
    eprintln!(
        "records={size} operation={operation} command_ms={:.3} render_median_ms={:.3} render_p95_ms={:.3} median_mutations={}",
        median.0, median.1, p95.1, median.2
    );
}

#[test]
#[ignore = "synthetic performance probe; run with --ignored --nocapture --test-threads=1"]
fn synthetic_tree_latency() {
    eprintln!(
        "debug_assertions={} samples=20; Rust only, excludes IPC/layout/paint",
        cfg!(debug_assertions)
    );
    for size in [100, 1_000, 10_000] {
        let mut dom = mounted();
        let mut ids = Vec::new();
        let mut folder_id = String::new();
        update(&mut dom, |state| {
            edit_unlocked(state, |u| {
                let folders: Vec<_> = (0..(size / 10))
                    .map(|i| Folder::new(format!("Folder {i:05}"), None))
                    .collect();
                folder_id = folders[0].id.clone();
                let entries: Vec<_> = (0..size)
                    .map(|i| {
                        let mut entry = Entry::new();
                        entry.title = format!("Fictional entry {i:05}");
                        entry.folder_id = Some(folders[i % folders.len()].id.clone());
                        ids.push(entry.id.clone());
                        entry
                    })
                    .collect();
                *u.vault.test_entries_mut() = entries;
                *u.vault.test_folders_mut() = folders;
                u.directories.collapsed.clear();
                u.select(ids[0].clone());
            })
        });
        let projections = tree::PROJECTIONS.get();
        let selection = (0..20)
            .map(|i| {
                sample(&mut dom, |state| {
                    edit_unlocked(state, |u| u.select(ids[i % ids.len()].clone()))
                })
            })
            .collect();
        report(size, "select", selection);
        let expansion = (0..20)
            .map(|_| {
                sample(&mut dom, |state| {
                    edit_unlocked(state, |u| u.directories.toggle(folder_id.clone()))
                })
            })
            .collect();
        report(size, "toggle", expansion);
        assert_eq!(
            tree::PROJECTIONS.get(),
            projections,
            "cached hierarchy still reused"
        );
        let filtering = (0..20)
            .map(|i| {
                sample(&mut dom, |state| {
                    edit_unlocked(state, |u| {
                        u.search = if i % 2 == 0 {
                            "Fictional".into()
                        } else {
                            "entry".into()
                        }
                    })
                })
            })
            .collect();
        report(size, "filter_same_rows", filtering);
        let filtering = (0..20)
            .map(|i| {
                sample(&mut dom, |state| {
                    edit_unlocked(state, |u| {
                        u.search = if i % 2 == 0 {
                            "missing".into()
                        } else {
                            "entry".into()
                        }
                    })
                })
            })
            .collect();
        report(size, "filter_remove_restore", filtering);
        update(&mut dom, |state| edit_unlocked(state, |u| u.search.clear()));
        let notification = (0..20)
            .map(|_| sample(&mut dom, |state| edit_unlocked(state, |_| {})))
            .collect();
        report(size, "session_notification", notification);
    }
}
