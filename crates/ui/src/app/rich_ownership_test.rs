//! Headless Workbench lifecycle observation. No native renderer or service access.
use super::Workbench;
use diffz_core::{domain::*, export::ContextExport, provider::*, registry::Registry, review::*};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, WindowOptions};
use std::{path::PathBuf, rc::Rc, sync::Arc};

struct Offline;
impl WorkbenchServices for Offline {
    fn open(&self, _: OpenRequest, _: Cancellation) -> Result<Opened, ServiceError> {
        panic!("unexpected open")
    }
    fn save_draft(&self, _: Draft) -> Result<u64, ServiceError> {
        panic!("unexpected write")
    }
    fn discard_draft(&self, _: DraftId, _: u64) -> Result<(), ServiceError> {
        panic!("unexpected discard")
    }
    fn save_view(&self, _: &SnapshotId, _: SavedView) -> Result<(), ServiceError> {
        Ok(())
    }
    fn recent(&self) -> Result<Vec<RecentSession>, ServiceError> {
        Ok(vec![])
    }
    fn prepare(
        &self,
        _: &SnapshotId,
        _: Vec<Draft>,
        _: Verdict,
        _: String,
    ) -> Result<PreparedReview, ServiceError> {
        panic!("unexpected prepare")
    }
    fn publish(&self, _: PreparedReview) -> Result<OutboxEntry, ServiceError> {
        panic!("unexpected publish")
    }
    fn outbox(&self) -> Result<Vec<OutboxEntry>, ServiceError> {
        Ok(vec![])
    }
    fn reconcile(&self, _: OperationId) -> Result<OutboxEntry, ServiceError> {
        panic!("unexpected reconcile")
    }
    fn export(&self, _: ContextExport, _: PathBuf) -> Result<(), ServiceError> {
        panic!("unexpected export")
    }
    fn writes_enabled(&self) -> bool {
        false
    }
    fn fresh_id(&self) -> String {
        "offline".into()
    }
}
fn opened(path: &str, body: &str) -> Opened {
    let patch = format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-old\n+{body}\n"
    );
    let report = diffz_core::patch::parse_patch(patch.as_bytes(), Default::default()).unwrap();
    Opened {
        snapshot: Snapshot::new(path.into(), report, None, vec![]),
        drafts: vec![],
        view: SavedView::default(),
    }
}
#[gpui_kit::test]
fn workbench_install_releases_prior_rich_payload_after_frame_retirement(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (window, workbench) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                Workbench::new(
                    Arc::new(Offline),
                    Arc::new(Registry::default()),
                    None,
                    None,
                    window,
                    cx,
                )
            })
        })
        .unwrap()
    });
    cx.update_window(window, |_, _, cx| {
        workbench.update(cx, |app, cx| {
            app.settings.rich = true;
            app.install(opened("large.md", &"paragraph ".repeat(4096)), cx);
            assert!(app.rich_active());
        });
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let payload = cx
        .update_window(window, |_, _, cx| {
            workbench.update(cx, |app, cx| {
                let payload = Rc::downgrade(
                    &app.rich_cache
                        .as_ref()
                        .expect("rich render populated cache")
                        .items,
                );
                app.install(opened("small.rs", "fn main() {}"), cx);
                assert!(!app.rich_active());
                assert!(
                    app.rich_cache.is_none(),
                    "install invalidates copied rich text"
                );
                payload
            })
        })
        .unwrap();
    // Two replacement frames retire the previously painted rich element.
    for _ in 0..2 {
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
    assert!(
        payload.upgrade().is_none(),
        "replacement frames release retired rich payload"
    );
}

#[gpui_kit::test]
fn file_transition_releases_rich_payload_but_same_file_keeps_cache(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (window, workbench) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                Workbench::new(
                    Arc::new(Offline),
                    Arc::new(Registry::default()),
                    None,
                    None,
                    window,
                    cx,
                )
            })
        })
        .unwrap()
    });

    let (rich, code) = cx
        .update_window(window, |_, _, cx| {
            workbench.update(cx, |app, cx| {
                app.settings.rich = true;
                let mut review = opened("large.md", &"paragraph ".repeat(4096));
                let mut code_review = opened("small.rs", "fn main() {}");
                let code = code_review.snapshot.patch.files.remove(0);
                let code_id = code.id.clone();
                let rich_id = review.snapshot.patch.files[0].id.clone();
                review.snapshot.patch.files.push(code);
                app.install(review, cx);
                (rich_id, code_id)
            })
        })
        .unwrap();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let payload = cx
        .update_window(window, |_, _, cx| {
            workbench.update(cx, |app, cx| {
                let items = Rc::downgrade(&app.rich_cache.as_ref().unwrap().items);
                app.select_file(rich, cx);
                assert!(
                    Rc::ptr_eq(
                        &items.upgrade().unwrap(),
                        &app.rich_cache.as_ref().unwrap().items
                    ),
                    "same-file selection preserves cache and list state"
                );
                app.select_file(code, cx);
                assert!(!app.rich_active());
                assert!(app.rich_cache.is_none());
                items
            })
        })
        .unwrap();
    for _ in 0..2 {
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
    assert!(
        payload.upgrade().is_none(),
        "file transition releases retired rich payload"
    );
}
