//! End-to-end `remind::` scan: a rule authored on a block comes back
//! out of `scan_reminders` with a resolved next fire, and a snooze
//! applied through the op log silences it on the same pass.
//!
//! The unit tests in `reminders::schedule` cover the math. This file
//! covers the wiring the clients actually consume: block property →
//! op log → workspace snooze table → sorted list.

use std::path::Path;

use chrono::{NaiveDate, NaiveDateTime};
use outl_actions::reminders::{load_fired_log, scan_reminders, snooze_until, take_due, FiredLog};
use outl_actions::{
    append_block, apply_page_md_with_sidecar, open_or_create_page, set_property, PageKind,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::property::PropValue;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use tempfile::TempDir;

fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, m, d)
        .expect("valid date")
        .and_hms_opt(h, min, 0)
        .expect("valid time")
}

/// A workspace on disk with one page holding `blocks`, each described
/// as `(text, Option<remind rule>)`. Returns the workspace, its HLC
/// generator, and the block ids in order.
fn workspace_with(
    root: &Path,
    blocks: &[(&str, Option<&str>)],
) -> (Workspace, HlcGenerator, Vec<outl_core::id::NodeId>) {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let storage = JsonlStorage::open(root.join("ops"), actor).expect("storage opens");
    let mut w = Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf()))
        .expect("workspace opens");

    let page = open_or_create_page(&mut w, &hlc, "tasks", "tasks", PageKind::Page).expect("page");
    let mut ids = Vec::new();
    for (text, rule) in blocks {
        let id = append_block(&mut w, &hlc, Some(page), Some(text)).expect("block");
        if let Some(rule) = rule {
            set_property(
                &mut w,
                &hlc,
                id,
                "remind",
                Some(PropValue::Text(rule.to_string())),
            )
            .expect("remind property");
        }
        ids.push(id);
    }
    // The sidecar is what carries the block ids the scan reads back —
    // without it the ids are position-derived and never match the
    // workspace's snooze table.
    apply_page_md_with_sidecar(&w, root, page).expect("projection");
    (w, hlc, ids)
}

#[test]
fn a_rule_in_markdown_comes_back_with_a_next_fire() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let (w, _hlc, _ids) = workspace_with(
        root,
        &[
            ("TODO ship it [[2026-12-12]]", Some("10am every 1h")),
            ("just a note", None),
        ],
    );

    let found = scan_reminders(&w, &FiredLog::new(), None, at(2026, 12, 12, 9, 0));

    assert_eq!(found.len(), 1, "only the block with a rule schedules");
    let r = &found[0];
    assert_eq!(r.page_slug, "tasks");
    assert_eq!(r.rule_text, "10am every 1h");
    assert_eq!(
        r.anchor_date,
        NaiveDate::from_ymd_opt(2026, 12, 12).unwrap()
    );
    assert_eq!(r.next_fire, Some(at(2026, 12, 12, 10, 0)));
    assert!(!r.done);
}

#[test]
fn an_unparseable_rule_schedules_nothing_and_costs_no_other_reminder() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let (w, _hlc, _ids) = workspace_with(
        root,
        &[
            ("TODO broken [[2026-12-12]]", Some("every 1h")),
            ("TODO fine [[2026-12-12]]", Some("10am")),
        ],
    );

    let found = scan_reminders(&w, &FiredLog::new(), None, at(2026, 12, 12, 9, 0));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].text, "fine [[2026-12-12]]");
}

#[test]
fn a_done_block_never_fires_again() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let (w, _hlc, _ids) = workspace_with(
        root,
        &[("DONE shipped it [[2026-12-12]]", Some("10am every 1h"))],
    );

    let found = scan_reminders(&w, &FiredLog::new(), None, at(2026, 12, 12, 9, 0));
    assert_eq!(found.len(), 1, "still listed, so the UI can show it");
    assert!(found[0].done);
    assert_eq!(found[0].next_fire, None, "but it never interrupts again");
}

#[test]
fn a_snooze_from_the_op_log_pushes_the_next_fire() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let (mut w, hlc, ids) = workspace_with(
        root,
        &[("TODO nag me [[2026-12-12]]", Some("10am every 1h"))],
    );

    let before = scan_reminders(&w, &FiredLog::new(), None, at(2026, 12, 12, 9, 0));
    assert_eq!(before[0].next_fire, Some(at(2026, 12, 12, 10, 0)));

    // The block id in the scan comes from the sidecar; it must be the
    // same node the snooze op targets, or the two never meet.
    assert_eq!(before[0].block_id, ids[0]);
    snooze_until(&mut w, &hlc, ids[0], at(2026, 12, 12, 14, 0)).expect("snooze applies");

    let after = scan_reminders(&w, &FiredLog::new(), None, at(2026, 12, 12, 9, 0));
    assert_eq!(after[0].next_fire, Some(at(2026, 12, 12, 14, 0)));
    assert!(after[0].snoozed_until_ms.is_some());
}

#[test]
fn the_list_is_sorted_by_next_fire_with_finished_entries_last() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let (w, _hlc, _ids) = workspace_with(
        root,
        &[
            ("TODO late [[2026-12-12]]", Some("4pm")),
            ("DONE finished [[2026-12-12]]", Some("11am")),
            ("TODO early [[2026-12-12]]", Some("10am")),
        ],
    );

    let found = scan_reminders(&w, &FiredLog::new(), None, at(2026, 12, 12, 9, 0));
    let order: Vec<&str> = found.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(
        order,
        vec![
            "early [[2026-12-12]]",
            "late [[2026-12-12]]",
            "finished [[2026-12-12]]",
        ]
    );
}

#[test]
fn two_dates_in_one_block_schedule_twice() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let (w, _hlc, _ids) = workspace_with(
        root,
        &[("TODO ping [[2026-12-12]] and [[2026-12-15]]", Some("10am"))],
    );

    let found = scan_reminders(&w, &FiredLog::new(), None, at(2026, 12, 12, 9, 0));
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].next_fire, Some(at(2026, 12, 12, 10, 0)));
    assert_eq!(found[1].next_fire, Some(at(2026, 12, 15, 10, 0)));
}

#[test]
fn a_rule_is_visible_before_the_md_is_ever_projected() {
    // Regression: the scan used to read the `.md`, which the GUI
    // clients write **asynchronously**. Author a reminder and open the
    // list a moment later and it wasn't there — which the user reads
    // as "it didn't save". The op log is the source of truth, so the
    // scan reads that; no projection is needed for a rule to count.
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let storage = JsonlStorage::open(root.join("ops"), actor).expect("storage opens");
    let mut w = Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf()))
        .expect("workspace opens");

    let page = open_or_create_page(&mut w, &hlc, "tasks", "tasks", PageKind::Page).expect("page");
    let id = append_block(
        &mut w,
        &hlc,
        Some(page),
        Some("TODO ship it [[2026-12-12]]"),
    )
    .expect("block");
    set_property(
        &mut w,
        &hlc,
        id,
        "remind",
        Some(PropValue::Text("10am".to_string())),
    )
    .expect("remind property");
    // Deliberately NO `apply_page_md_with_sidecar` here.

    let found = scan_reminders(&w, &FiredLog::new(), None, at(2026, 12, 12, 9, 0));
    assert_eq!(found.len(), 1, "the op log alone is enough");
    assert_eq!(found[0].block_id, id);
    assert_eq!(found[0].next_fire, Some(at(2026, 12, 12, 10, 0)));
}

#[test]
fn a_deleted_block_stops_nagging() {
    // Delete is `Move(node, TRASH_ROOT)`, so the block keeps its
    // `remind::` property. It must still drop out of the scan — a
    // reminder for something the user threw away is the worst kind of
    // notification.
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let (mut w, hlc, ids) = workspace_with(root, &[("TODO ship it [[2026-12-12]]", Some("10am"))]);
    assert_eq!(
        scan_reminders(&w, &FiredLog::new(), None, at(2026, 12, 12, 9, 0)).len(),
        1
    );

    outl_actions::delete(&mut w, &hlc, ids[0]).expect("delete");

    assert!(scan_reminders(&w, &FiredLog::new(), None, at(2026, 12, 12, 9, 0)).is_empty());
}

#[test]
fn take_due_fires_once_then_stays_quiet() {
    // The delivery contract: a due reminder comes out once, and the
    // device-local fired log keeps the next poll from re-buzzing. The
    // TUI ticks this every event-loop pass and the GUI clients every
    // 30s, so a second call returning the same row would be a storm.
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let (w, _hlc, _ids) = workspace_with(root, &[("TODO ship it [[2026-12-12]]", Some("10am"))]);

    let first = take_due(&w, root, None, at(2026, 12, 12, 10, 0));
    assert_eq!(first.len(), 1, "the 10am fire is due at 10:00");

    let second = take_due(&w, root, None, at(2026, 12, 12, 10, 0));
    assert!(second.is_empty(), "a single-shot rule must not fire twice");

    // And the memory of it is on disk, so a restart doesn't re-buzz.
    assert_eq!(load_fired_log(root).len(), 1);
}

#[test]
fn take_due_stays_quiet_before_the_anchor() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let (w, _hlc, _ids) = workspace_with(root, &[("TODO ship it [[2026-12-12]]", Some("10am"))]);

    assert!(take_due(&w, root, None, at(2026, 12, 12, 9, 59)).is_empty());
    assert_eq!(take_due(&w, root, None, at(2026, 12, 12, 10, 0)).len(), 1);
}

#[test]
fn a_repeating_rule_fires_again_after_its_interval() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let (w, _hlc, _ids) = workspace_with(
        root,
        &[(
            "TODO nag me [[2026-12-12]]",
            Some("10am every 1h until DONE"),
        )],
    );

    assert_eq!(take_due(&w, root, None, at(2026, 12, 12, 10, 0)).len(), 1);
    assert!(take_due(&w, root, None, at(2026, 12, 12, 10, 30)).is_empty());
    assert_eq!(take_due(&w, root, None, at(2026, 12, 12, 11, 0)).len(), 1);
}

#[test]
fn a_workspace_with_no_rules_never_touches_the_fired_log() {
    // Delivery defaults on and every client polls on a timer, so the
    // zero-reminder workspace is the common case. It used to pay for a
    // full scan plus a file read on every tick, under the same
    // workspace lock a block edit needs.
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let (w, _hlc, _ids) = workspace_with(root, &[("TODO no reminder here", None)]);

    assert!(take_due(&w, root, None, at(2026, 12, 12, 10, 0)).is_empty());
    assert!(
        !outl_actions::reminders::fired_log_path(root).exists(),
        "a sweep that found nothing must not have written a fired log"
    );
}
